#!/usr/bin/env python3
"""HTTP sidecar: CDC topics → embeddings → PGVector, built on pydebeziumai (ADR 0018).

  GET    /health
  GET    /v1/capability                  versions, providers, Kafka/registry endpoints
  GET    /v1/topics[?prefix=a.,b.]       data topics with retained message counts
  GET    /v1/syncs                       status of every running sync
  PUT    /v1/syncs/{id}                  desired spec (secrets included, memory only); unchanged
                                         revision+generation = no-op
  GET    /v1/syncs/{id}                  status
  DELETE /v1/syncs/{id}[?forget=true]   stop; forget also drops state, dead letters, groups
  POST   /v1/collections/drop        {target} → delete a collection and its vectors
  POST   /v1/syncs/{id}/search           {query, k?, filter?} → hits
  GET    /v1/syncs/{id}/dead-letters     ?limit=  → {items, total}
  DELETE /v1/syncs/{id}/dead-letters     clear
  POST   /v1/test                        {embedding?, target?} → {ok, checks[]}
  POST   /v1/embedding-models            {embedding} → {models[]}
  POST   /v1/preview                     {topics, projection, id_strategy, delete_mode, sample?}

The orchestrator owns the specs (rqlite) and re-sends them after a sidecar restart.
"""

from __future__ import annotations

import json
import os
import platform
import signal
import sys
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from importlib import metadata
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse

import bridge
import chunking
import docling_tx
from decode import Decoder, DecodeError, RegistryUnavailable
from embeddings import DEFAULT_MODELS, PROVIDERS, EmbeddingUnavailable, ProviderEmbeddings
from engine import BadSpec, DeadLetters, InfraError, Registry, Settings, Shaper, StateFile, drop_collection, pg_conninfo, validate_spec

WORK_ROOT = Path(os.environ.get("VECTORSYNC_WORK_ROOT", "/var/lib/vectorsync"))
API_TOKEN = os.environ.get("VECTORSYNC_API_TOKEN", "").strip()
HOST = os.environ.get("VECTORSYNC_API_HOST", "0.0.0.0")
PORT = int(os.environ.get("VECTORSYNC_API_PORT", "8092"))
DLQ_MAX = max(10, int(os.environ.get("VECTORSYNC_DLQ_MAX", "1000")))
SECRET_KEYS = {"password", "api_key"}


class NotFound(Exception):
    pass


def _ver(dist: str) -> str | None:
    try:
        return metadata.version(dist)
    except metadata.PackageNotFoundError:
        return None


def capability(settings: Settings) -> dict[str, Any]:
    return {
        "service": "cdc-vectorsync",
        "tool_id": "vectorsync",
        "versions": {
            "pydebeziumai": _ver("pydebeziumai"),
            "langchain_postgres": _ver("langchain-postgres"),
            "langchain_core": _ver("langchain-core"),
            "confluent_kafka": _ver("confluent-kafka"),
            "python": platform.python_version(),
        },
        "providers": list(PROVIDERS),
        "default_models": DEFAULT_MODELS,
        "kafka_bootstrap": settings.bootstrap,
        "schema_registry": settings.registry,
        "targets": ["pgvector"],
        "docling": {
            "available": docling_tx.AVAILABLE,
            "version": docling_tx.version(),
            "max_mb": docling_tx.MAX_MB,
            "max_pages": docling_tx.MAX_PAGES,
            "formats": ["pdf", "docx", "pptx", "xlsx", "html", "md", "csv", "png", "jpg", "tiff"],
        },
        "chunking": {"default_max_chars": chunking.DEFAULT_MAX_CHARS, "default_overlap": chunking.DEFAULT_OVERLAP},
    }


# ── connection tests ─────────────────────────────────────────────────────────────────────

def _check(name: str, ok: bool, detail: str, hint: str | None = None, **extra: Any) -> dict[str, Any]:
    out = {"name": name, "ok": ok, "detail": detail}
    if hint:
        out["hint"] = hint
    out.update(extra)
    return out


def pg_hint(text: str) -> str | None:
    s = text.lower()
    if "password authentication failed" in s or "no password supplied" in s:
        return "The server rejected the login. Check the username and password."
    if "could not translate host name" in s or "name or service not known" in s or "nodename nor servname" in s:
        return "The host does not resolve from the sidecar container. For this machine use host.docker.internal."
    if "connection refused" in s or "timeout" in s or "timed out" in s:
        return "Nothing answered on that host/port from the sidecar container. Check host, port and firewall."
    if "does not exist" in s and "database" in s:
        return "The database does not exist on that server."
    if "pg_hba.conf" in s:
        return "pg_hba.conf rejects the sidecar's address. Allow it, or change sslmode."
    return None


def test_target(t: dict[str, Any]) -> list[dict[str, Any]]:
    import psycopg

    checks = []
    try:
        with psycopg.connect(**pg_conninfo(t)) as conn:
            ver = conn.execute("SHOW server_version").fetchone()[0]
            checks.append(_check("postgres", True, f"Connected — PostgreSQL {ver}"))
            installed = conn.execute("SELECT extversion FROM pg_extension WHERE extname = 'vector'").fetchone()
            available = conn.execute("SELECT default_version FROM pg_available_extensions WHERE name = 'vector'").fetchone()
            if installed:
                checks.append(_check("pgvector", True, f"pgvector {installed[0]} is installed"))
            elif available:
                can = conn.execute(
                    "SELECT rolsuper OR has_database_privilege(current_user, current_database(), 'CREATE') FROM pg_roles WHERE rolname = current_user"
                ).fetchone()
                if can and can[0]:
                    checks.append(_check("pgvector", True, f"pgvector {available[0]} is available; it will be enabled on first sync"))
                else:
                    checks.append(_check("pgvector", False, "pgvector is available but not enabled", "Run CREATE EXTENSION vector; as a superuser in this database."))
            else:
                checks.append(_check("pgvector", False, "pgvector is not installed on this server", "Use a Postgres image with pgvector (e.g. pgvector/pgvector:pg17) or install the extension."))
            coll = (t.get("collection") or "").strip()
            if coll:
                exists = conn.execute("SELECT to_regclass('langchain_pg_collection') IS NOT NULL").fetchone()[0]
                n = None
                if exists:
                    row = conn.execute(
                        "SELECT count(e.*) FROM langchain_pg_collection c LEFT JOIN langchain_pg_embedding e ON e.collection_id = c.uuid WHERE c.name = %s GROUP BY c.uuid",
                        (coll,),
                    ).fetchone()
                    n = row[0] if row else None
                detail = f"Collection '{coll}' exists with {n} document(s)" if n is not None else f"Collection '{coll}' will be created"
                checks.append(_check("collection", True, detail, documents=n))
    except Exception as exc:  # noqa: BLE001
        msg = str(exc).strip().splitlines()[0] if str(exc).strip() else type(exc).__name__
        checks.append(_check("postgres", False, msg, pg_hint(msg)))
    return checks


def test_embedding(cfg: dict[str, Any]) -> list[dict[str, Any]]:
    try:
        emb = ProviderEmbeddings(cfg)
    except ValueError as exc:
        return [_check("embedding", False, str(exc))]
    try:
        p = emb.probe()
        note = " (hash vectors are not semantic: use only for demos)" if emb.provider == "hash" else ""
        return [_check("embedding", True, f"{emb.model}: {p['dimensions']} dimensions in {p['latency_ms']} ms{note}", **p)]
    except EmbeddingUnavailable as exc:
        s = str(exc)
        hint = None
        if "404" in s and emb.provider == "ollama":
            hint = f"Ollama does not have '{emb.model}'. Run: ollama pull {emb.model}"
        elif "401" in s or "403" in s:
            hint = "The API key was rejected."
        elif "refused" in s.lower() or "timed out" in s.lower() or "name" in s.lower():
            hint = "The embedding server is not reachable from the sidecar container (for Ollama on this machine: http://host.docker.internal:11434)."
        return [_check("embedding", False, s, hint)]


# ── preview ──────────────────────────────────────────────────────────────────────────────

def preview(body: dict[str, Any], settings: Settings) -> dict[str, Any]:
    from confluent_kafka import Consumer, TopicPartition

    topics = [t for t in (body.get("topics") or []) if isinstance(t, str) and t]
    if not topics:
        raise BadSpec("topics: pick at least one table")
    sample = max(1, min(int(body.get("sample") or 3), 20))
    spec = {"projection": body.get("projection") or {}, "id_strategy": body.get("id_strategy") or {}, "name": "preview"}
    for name, cfg in [("default", spec["projection"].get("default"))] + list((spec["projection"].get("tables") or {}).items()):
        err = bridge.check_template((cfg or {}).get("content_template"))
        if err:
            raise BadSpec(f"{name}: {err}")
    builder = bridge.make_builder(spec)
    shaper = Shaper(spec)
    soft = (body.get("delete_mode") or "hard") == "soft"
    for name, cfg in (spec["projection"].get("tables") or {}).items():
        if ((cfg or {}).get("document") or {}).get("column") and not docling_tx.AVAILABLE:
            raise BadSpec(f"{name}: Docling is not in this cdc-vectorsync image (set VECTORSYNC_DOCLING=true and redeploy)")
    decoder = Decoder(settings.registry, settings.registry_auth)
    consumer = Consumer({
        "bootstrap.servers": settings.bootstrap,
        "group.id": f"cdc-vectorsync-preview-{uuid.uuid4().hex[:8]}",
        "enable.auto.commit": False,
        "allow.auto.create.topics": False,
        **settings.kafka_extra,
    })
    out = []
    try:
        md = consumer.list_topics(timeout=8)
        for topic in topics:
            entry: dict[str, Any] = {"topic": topic, "table": topic.split(".")[-1], "columns": [], "records": [], "messages": 0}
            tmeta = md.topics.get(topic)
            if tmeta is None or tmeta.error is not None:
                entry["error"] = "Topic does not exist yet — start the pipeline (or wait for its first snapshot)."
                out.append(entry)
                continue
            assign = []
            total = 0
            for p in tmeta.partitions:
                low, high = consumer.get_watermark_offsets(TopicPartition(topic, p), timeout=5)
                total += max(0, high - low)
                if high > low:
                    assign.append(TopicPartition(topic, p, max(low, high - sample)))
            entry["messages"] = total
            if not assign:
                entry["error"] = "The topic is empty — nothing to preview yet."
                out.append(entry)
                continue
            consumer.assign(assign)
            columns: dict[str, None] = {}
            deadline = time.monotonic() + 6
            seen = 0
            while seen < sample * len(assign) and time.monotonic() < deadline:
                msgs = consumer.consume(num_messages=sample * len(assign), timeout=1)
                for m in msgs:
                    if m.error():
                        continue
                    seen += 1
                    rec: dict[str, Any] = {"partition": m.partition(), "offset": m.offset()}
                    try:
                        key = decoder.decode(m.key())
                        val = decoder.decode(m.value())
                        ev = bridge.to_event(topic, m.partition(), key, val)
                        if ev is None:
                            rec["skipped"] = "tombstone / schema event"
                        else:
                            for col in (ev.payload.current_row or {}):
                                columns[col] = None
                            rec["op"] = ev.payload.op
                            rec["doc_id"] = bridge.doc_id(builder, ev)
                            if ev.payload.op == "d" and not soft:
                                rec["action"] = "delete"
                            else:
                                doc = bridge.build(builder, ev, soft_delete=ev.payload.op == "d")
                                change = bridge.Change(ev, rec["doc_id"], ev.payload.op, topic, m.partition(), m.offset())
                                docs = shaper.expand(change, doc, soft_delete_row=ev.payload.op == "d")
                                rec["action"] = "upsert"
                                rec["content"] = docs[0].page_content
                                rec["chars"] = len(docs[0].page_content)
                                rec["metadata"] = docs[0].metadata
                                if len(docs) > 1 or docs[0].metadata.get("_content") == "document":
                                    rec["chunks"] = len(docs)
                                    rec["chunk_samples"] = [d.page_content for d in docs[:3]]
                    except (DecodeError, bridge.DataError) as exc:
                        rec["error"] = str(exc)
                    except RegistryUnavailable as exc:
                        entry["error"] = str(exc)
                        break
                    entry["records"].append(rec)
            consumer.unassign()
            entry["columns"] = list(columns)
            entry["records"].sort(key=lambda r: (r["partition"], r["offset"]), reverse=True)
            entry["records"] = entry["records"][:sample]
            out.append(entry)
    finally:
        consumer.close()
    return {"topics": out}


def list_topics(settings: Settings, prefixes: list[str]) -> list[dict[str, Any]]:
    """Data topics (optionally under some prefixes) with retained message counts. `trimmed`
    means retention already deleted the oldest records, so a rebuild cannot replay everything."""
    from confluent_kafka import Consumer, TopicPartition

    c = Consumer({"bootstrap.servers": settings.bootstrap, "group.id": "cdc-vectorsync-inventory", "enable.auto.commit": False, **settings.kafka_extra})
    try:
        md = c.list_topics(timeout=8)
        out = []
        for name, t in sorted(md.topics.items()):
            if name.startswith("_") or name.endswith("-dlq") or (prefixes and not any(name.startswith(p) for p in prefixes)):
                continue
            retained, trimmed = 0, False
            for p in t.partitions:
                low, high = c.get_watermark_offsets(TopicPartition(name, p), timeout=5)
                retained += max(0, high - low)
                trimmed = trimmed or low > 0
            out.append({"name": name, "partitions": len(t.partitions), "messages": retained, "trimmed": trimmed})
        return out
    finally:
        c.close()


# ── HTTP ─────────────────────────────────────────────────────────────────────────────────

def redact(value: Any) -> Any:
    if isinstance(value, dict):
        return {k: ("********" if k in SECRET_KEYS and v else redact(v)) for k, v in value.items()}
    if isinstance(value, list):
        return [redact(v) for v in value]
    return value


def make_handler(registry: Registry, settings: Settings) -> type[BaseHTTPRequestHandler]:
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt: str, *args: Any) -> None:
            return

        def _auth_ok(self) -> bool:
            if not API_TOKEN:
                return True
            auth = self.headers.get("Authorization", "")
            return (auth.startswith("Bearer ") and auth[7:].strip() == API_TOKEN) or self.headers.get("X-API-Token", "").strip() == API_TOKEN

        def _json(self, code: int, payload: Any) -> None:
            body = json.dumps(payload, default=str).encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _body(self) -> dict[str, Any]:
            length = int(self.headers.get("Content-Length", "0") or "0")
            raw = self.rfile.read(length) if length else b"{}"
            try:
                data = json.loads(raw.decode("utf-8") or "{}")
            except (UnicodeDecodeError, json.JSONDecodeError):
                raise BadSpec("request body must be JSON") from None
            if not isinstance(data, dict):
                raise BadSpec("request body must be a JSON object")
            return data

        def _route(self, method: str) -> None:
            url = urlparse(self.path)
            parts = [p for p in url.path.split("/") if p]
            if parts == ["health"] and method in ("GET", "HEAD"):
                self._json(200, {"status": "ok", "syncs": len(registry.list())})
                return
            if not self._auth_ok():
                self._json(401, {"error": "unauthorized"})
                return
            q = {k: v[-1] for k, v in parse_qs(url.query).items()}
            try:
                self._dispatch(method, parts, q)
            except BadSpec as exc:
                self._json(400, {"error": str(exc)})
            except NotFound as exc:
                self._json(404, {"error": str(exc)})
            except InfraError as exc:
                self._json(503, {"error": str(exc)})
            except Exception as exc:  # noqa: BLE001
                self._json(500, {"error": f"{type(exc).__name__}: {exc}"})

        def _sync(self, sid: str) -> Any:
            s = registry.get(sid)
            if s is None:
                raise NotFound(f"sync {sid} is not running on this sidecar")
            return s

        def _dispatch(self, method: str, parts: list[str], q: dict[str, str]) -> None:
            if parts == ["v1", "capability"] or parts == ["v1", "discover"]:
                self._json(200, capability(settings))
            elif parts == ["v1", "topics"] and method == "GET":
                prefixes = [p for p in (q.get("prefix") or "").split(",") if p]
                self._json(200, {"topics": list_topics(settings, prefixes)})
            elif parts == ["v1", "syncs"] and method == "GET":
                self._json(200, {"syncs": registry.list()})
            elif len(parts) == 3 and parts[:2] == ["v1", "syncs"]:
                sid = parts[2]
                if method == "PUT":
                    spec = self._body()
                    spec["id"] = sid
                    self._json(200, registry.put(spec))
                elif method == "GET":
                    self._json(200, self._sync(sid).snapshot())
                elif method == "DELETE":
                    found = registry.delete(sid, forget=q.get("forget") == "true")
                    self._json(200, {"id": sid, "stopped": found})
                else:
                    self._json(405, {"error": "method not allowed"})
            elif len(parts) == 4 and parts[:2] == ["v1", "syncs"] and parts[3] == "search" and method == "POST":
                body = self._body()
                query = str(body.get("query") or "").strip()
                if not query:
                    raise BadSpec("query is required")
                k = max(1, min(int(body.get("k") or 5), 50))
                flt = body.get("filter") if isinstance(body.get("filter"), dict) else None
                t0 = time.monotonic()
                hits = self._sync(parts[2]).search(query, k, flt)
                self._json(200, {"hits": hits, "took_ms": round((time.monotonic() - t0) * 1000, 1)})
            elif len(parts) == 4 and parts[:2] == ["v1", "syncs"] and parts[3] == "dead-letters":
                if method == "GET":
                    limit = max(1, min(int(q.get("limit") or 100), 1000))
                    self._json(200, {"items": registry.dlq.list(parts[2], limit), "total": registry.dlq.count(parts[2])})
                elif method == "DELETE":
                    self._json(200, {"cleared": registry.dlq.clear(parts[2])})
                else:
                    self._json(405, {"error": "method not allowed"})
            elif parts == ["v1", "test"] and method == "POST":
                body = self._body()
                checks: list[dict[str, Any]] = []
                if isinstance(body.get("embedding"), dict):
                    checks += test_embedding(body["embedding"])
                if isinstance(body.get("target"), dict):
                    checks += test_target(body["target"])
                if not checks:
                    raise BadSpec("send embedding and/or target")
                self._json(200, {"ok": all(c["ok"] for c in checks), "checks": checks})
            elif parts == ["v1", "embedding-models"] and method == "POST":
                cfg = self._body().get("embedding") or {}
                try:
                    self._json(200, {"models": ProviderEmbeddings(cfg).list_models()})
                except (EmbeddingUnavailable, ValueError) as exc:
                    self._json(200, {"models": [], "error": str(exc)})
            elif parts == ["v1", "preview"] and method == "POST":
                self._json(200, preview(self._body(), settings))
            elif parts == ["v1", "collections", "drop"] and method == "POST":
                target = self._body().get("target") or {}
                try:
                    self._json(200, {"dropped": True, "documents": drop_collection(target)})
                except BadSpec:
                    raise
                except Exception as exc:  # noqa: BLE001
                    msg = str(exc).strip().splitlines()[0] if str(exc).strip() else type(exc).__name__
                    raise InfraError(msg) from exc
            elif parts == ["v1", "validate"] and method == "POST":
                validate_spec(self._body())
                self._json(200, {"ok": True})
            else:
                self._json(404, {"error": "not found"})

        def do_GET(self) -> None:  # noqa: N802
            self._route("GET")

        def do_HEAD(self) -> None:  # noqa: N802
            self._route("HEAD")

        def do_POST(self) -> None:  # noqa: N802
            self._route("POST")

        def do_PUT(self) -> None:  # noqa: N802
            self._route("PUT")

        def do_DELETE(self) -> None:  # noqa: N802
            self._route("DELETE")

    return Handler


def main() -> None:
    (WORK_ROOT / "dead-letters").mkdir(parents=True, exist_ok=True)
    settings = Settings()
    registry = Registry(settings, StateFile(WORK_ROOT / "state.json"), DeadLetters(WORK_ROOT / "dead-letters", DLQ_MAX))
    server = ThreadingHTTPServer((HOST, PORT), make_handler(registry, settings))

    def shutdown(*_: Any) -> None:
        threading.Thread(target=server.shutdown, daemon=True).start()
        registry.stop_all()

    signal.signal(signal.SIGTERM, shutdown)
    print(f"cdc-vectorsync listening on {HOST}:{PORT} (pydebeziumai {_ver('pydebeziumai')}, kafka {settings.bootstrap})", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    main()
