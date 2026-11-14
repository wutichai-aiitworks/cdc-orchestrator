"""Sync workers for cdc-vectorsync (ADR 0018).

One thread per sync: a Kafka consumer on the pipeline's existing Debezium topics → pydebeziumai
documents → embeddings → PGVector. Delivery is at-least-once and idempotent (document ids are
stable per row, writes are upserts):

  * data errors (undecodable record, no key, template/build failure, 4xx from the embedding
    server for one document) → dead-letter file, offset committed, sync keeps going;
  * infrastructure errors (Kafka, registry, embedding server, Postgres) → the batch is rewound
    (seek to its first offsets), the worker backs off and retries; nothing is committed.

Specs (with secrets) arrive from the orchestrator and live in memory only. The volume keeps
non-secret state: the generation each collection was built for, and dead-letter files.
"""

from __future__ import annotations

import json
import os
import random
import re
import threading
import time
import traceback
import urllib.parse
from collections import deque
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

import bridge
import chunking as chunking_mod
import docling_tx
from decode import Decoder, DecodeError, RegistryUnavailable
from embeddings import PROVIDERS, EmbeddingUnavailable, ProviderEmbeddings

COLLECTION_RE = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.\-]{0,62}$")
TOPIC_RE = re.compile(r"^[A-Za-z0-9._\-]{1,249}$")
CLIENT_ERROR_RE = re.compile(r"HTTP (400|413|422)\b")


class BadSpec(Exception):
    pass


class InfraError(Exception):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def _env_bool(name: str, default: bool) -> bool:
    v = os.environ.get(name)
    return default if v is None else v.strip().lower() in ("1", "true", "yes", "on")


# ── spec ─────────────────────────────────────────────────────────────────────────────────

def pg_url(t: dict[str, Any]) -> str:
    user = urllib.parse.quote(str(t.get("username") or ""), safe="")
    pw = urllib.parse.quote(str(t.get("password") or ""), safe="")
    auth = f"{user}:{pw}@" if pw else f"{user}@"
    host = str(t.get("host") or "").strip()
    port = str(t.get("port") or "5432").strip()
    db = urllib.parse.quote(str(t.get("database") or ""), safe="")
    q = f"?sslmode={urllib.parse.quote(t['sslmode'])}" if t.get("sslmode") else ""
    return f"postgresql+psycopg://{auth}{host}:{port}/{db}{q}"


def pg_conninfo(t: dict[str, Any]) -> dict[str, Any]:
    out = {
        "host": str(t.get("host") or "").strip(),
        "port": int(str(t.get("port") or "5432")),
        "dbname": str(t.get("database") or ""),
        "user": str(t.get("username") or ""),
        "password": str(t.get("password") or ""),
        "connect_timeout": 8,
    }
    if t.get("sslmode"):
        out["sslmode"] = t["sslmode"]
    return out


def validate_spec(spec: dict[str, Any], need_topics: bool = True) -> None:
    topics = spec.get("topics") or []
    if need_topics:
        if not isinstance(topics, list) or not topics:
            raise BadSpec("topics: pick at least one table")
        for t in topics:
            if not isinstance(t, str) or not TOPIC_RE.match(t):
                raise BadSpec(f"topics: {t!r} is not a topic name")
    emb = spec.get("embedding") or {}
    if (emb.get("provider") or "ollama") not in PROVIDERS:
        raise BadSpec(f"embedding.provider must be one of {', '.join(PROVIDERS)}")
    tgt = spec.get("target") or {}
    for f in ("host", "database", "username"):
        if not str(tgt.get(f) or "").strip():
            raise BadSpec(f"target.{f} is required")
    coll = str(tgt.get("collection") or "")
    if not COLLECTION_RE.match(coll):
        raise BadSpec("target.collection: letters, digits, _ . - (max 63, not starting with . or -)")
    proj = spec.get("projection") or {}
    for name, cfg in [("default", proj.get("default"))] + list((proj.get("tables") or {}).items()):
        cfg = cfg or {}
        err = bridge.check_template(cfg.get("content_template"))
        if err:
            raise BadSpec(f"projection {name}: {err}")
        ch = cfg.get("chunking") or {}
        if ch:
            mc = int(ch.get("max_chars") or chunking_mod.DEFAULT_MAX_CHARS)
            ov = int(ch.get("overlap") or 0)
            if not 200 <= mc <= 20000:
                raise BadSpec(f"projection {name}: chunk size must be 200-20000 characters")
            if not 0 <= ov <= mc // 2:
                raise BadSpec(f"projection {name}: overlap must be 0 to half the chunk size")
        doc = cfg.get("document") or {}
        if doc.get("column"):
            if not docling_tx.AVAILABLE:
                raise BadSpec(f"projection {name}: Docling is not in this cdc-vectorsync image (set VECTORSYNC_DOCLING=true and redeploy)")
            if not isinstance(doc["column"], str):
                raise BadSpec(f"projection {name}: document.column must be a column name")
    ids = spec.get("id_strategy") or {}
    if ids.get("kind") == "composite" and not ids.get("pk_fields"):
        raise BadSpec("id_strategy composite needs pk_fields")
    if (spec.get("delete_mode") or "hard") not in ("hard", "soft"):
        raise BadSpec("delete_mode must be hard or soft")


# ── state on disk (no secrets) ───────────────────────────────────────────────────────────

class StateFile:
    def __init__(self, path: Path) -> None:
        self.path = path
        self._lock = threading.Lock()

    def _read(self) -> dict[str, Any]:
        try:
            return json.loads(self.path.read_text())
        except (FileNotFoundError, json.JSONDecodeError):
            return {}

    def get(self, sync_id: str) -> dict[str, Any]:
        with self._lock:
            return dict(self._read().get(sync_id) or {})

    def set(self, sync_id: str, **fields: Any) -> None:
        with self._lock:
            data = self._read()
            data.setdefault(sync_id, {}).update(fields)
            tmp = self.path.with_suffix(".tmp")
            tmp.write_text(json.dumps(data, indent=1))
            tmp.replace(self.path)

    def drop(self, sync_id: str) -> None:
        with self._lock:
            data = self._read()
            if data.pop(sync_id, None) is not None:
                tmp = self.path.with_suffix(".tmp")
                tmp.write_text(json.dumps(data, indent=1))
                tmp.replace(self.path)


class DeadLetters:
    """Bounded JSONL per sync. Holds where the record is (topic/partition/offset), its key and
    the error — never the row values, which can be read from the topic when needed."""

    def __init__(self, root: Path, max_items: int) -> None:
        self.root = root
        self.max_items = max_items
        self._lock = threading.Lock()

    def _path(self, sync_id: str) -> Path:
        return self.root / f"{sync_id}.jsonl"

    def add(self, sync_id: str, items: list[dict[str, Any]]) -> None:
        if not items:
            return
        with self._lock:
            p = self._path(sync_id)
            lines = p.read_text().splitlines() if p.exists() else []
            lines.extend(json.dumps(i, default=str) for i in items)
            lines = lines[-self.max_items :]
            p.write_text("\n".join(lines) + "\n")

    def list(self, sync_id: str, limit: int) -> list[dict[str, Any]]:
        with self._lock:
            p = self._path(sync_id)
            if not p.exists():
                return []
            lines = p.read_text().splitlines()[-limit:]
        return [json.loads(x) for x in reversed(lines) if x.strip()]

    def count(self, sync_id: str) -> int:
        with self._lock:
            p = self._path(sync_id)
            return sum(1 for x in p.read_text().splitlines() if x.strip()) if p.exists() else 0

    def clear(self, sync_id: str) -> int:
        with self._lock:
            p = self._path(sync_id)
            n = sum(1 for x in p.read_text().splitlines() if x.strip()) if p.exists() else 0
            p.unlink(missing_ok=True)
            return n


# ── batch apply (pure: testable without Kafka/Postgres) ──────────────────────────────────

def compact(changes: list[bridge.Change]) -> list[bridge.Change]:
    """Last change per document id wins (in batch order)."""
    latest: dict[str, bridge.Change] = {}
    for c in changes:
        latest.pop(c.doc_id, None)
        latest[c.doc_id] = c
    return list(latest.values())


def _strip_nul(doc: Any) -> Any:
    # Postgres text cannot hold NUL; a single bad byte would otherwise stall the sync forever.
    if "\x00" in doc.page_content:
        doc.page_content = doc.page_content.replace("\x00", "")
    for k, v in list(doc.metadata.items()):
        if isinstance(v, str) and "\x00" in v:
            doc.metadata[k] = v.replace("\x00", "")
    return doc


class Shaper:
    """How one row becomes documents (per table): one vector, text chunks, or a Docling
    document in a column turned into chunks. Every document carries `_parent_id` = the row's id,
    so updates and deletes can remove all of a row's vectors whatever its chunk count was."""

    def __init__(self, spec: dict[str, Any]) -> None:
        proj = spec.get("projection") or {}
        self.default = proj.get("default") or {}
        self.tables = proj.get("tables") or {}

    def cfg(self, table: str) -> dict[str, Any]:
        return self.tables.get(table) or self.default

    def expand(self, change: bridge.Change, base: Any, soft_delete_row: bool) -> list[Any]:
        from langchain_core.documents import Document

        cfg = self.cfg(change.event.table_name)
        chunking = cfg.get("chunking") or {}
        doc_cfg = cfg.get("document") or {}
        max_chars = int(chunking.get("max_chars") or chunking_mod.DEFAULT_MAX_CHARS)
        overlap = int(chunking.get("overlap") if chunking.get("overlap") is not None else chunking_mod.DEFAULT_OVERLAP)
        base.metadata["_parent_id"] = change.doc_id
        if soft_delete_row:
            return [base]  # a deleted row keeps one marked tombstone, not its chunks
        texts: list[str]
        extra: dict[str, Any] = {}
        if doc_cfg.get("column"):
            row = change.event.payload.current_row or {}
            col = doc_cfg["column"]
            if col not in row:
                raise bridge.DataError(f"document column {col!r} is not in the row")
            name_col = doc_cfg.get("filename_column")
            fname = str(row.get(name_col) or "") if name_col else None
            try:
                texts, extra = docling_tx.to_chunks(row.get(col), fname, max_chars, overlap)
            except docling_tx.DocumentError as exc:
                raise bridge.DataError(str(exc)) from exc
            if doc_cfg.get("include_row_text") and base.page_content:
                texts = [f"{base.page_content}\n\n{t}" for t in texts]
            extra["_content"] = "document"
        elif chunking.get("enabled"):
            texts = chunking_mod.split_text(base.page_content, max_chars, overlap) or [base.page_content]
            extra["_content"] = "text"
        else:
            return [base]
        if len(texts) == 1 and not doc_cfg.get("column"):
            return [base]
        out = []
        for n, t in enumerate(texts):
            meta = {**base.metadata, **extra, "_chunk": n, "_chunks": len(texts)}
            out.append(_strip_nul(Document(page_content=t, metadata=meta, id=f"{change.doc_id}#c{n}")))
        return out


def apply_batch(
    changes: list[bridge.Change],
    builder: Any,
    adapter: Any,
    soft_delete: bool,
    dead: Callable[[bridge.Change, str], None],
    shaper: Shaper | None = None,
) -> dict[str, int]:
    """Write one batch. Data problems go to `dead`; anything else raises InfraError.

    `adapter` must offer upsert_batch, upsert, delete_batch and delete_parents (VectorStore)."""
    shaper = shaper or Shaper({})
    stats = {"upserted": 0, "deleted": 0, "chunks": 0}
    deletes: list[str] = []
    upserts: list[tuple[bridge.Change, list[Any]]] = []
    for c in compact(changes):
        if c.op == "d" and not soft_delete:
            deletes.append(c.doc_id)
            continue
        try:
            base = _strip_nul(bridge.build(builder, c.event, soft_delete=c.op == "d"))
            upserts.append((c, shaper.expand(c, base, soft_delete_row=c.op == "d")))
        except bridge.DataError as exc:
            dead(c, str(exc))
    try:
        if deletes:
            adapter.delete_parents(deletes)
            adapter.delete_batch(deletes)  # vectors written before _parent_id existed
            stats["deleted"] = len(deletes)
        if upserts:
            # Old chunks of these rows go first: a shorter new version must not leave extras.
            adapter.delete_parents([c.doc_id for c, _ in upserts])
    except Exception as exc:  # noqa: BLE001
        raise InfraError(f"vector store delete: {exc}") from exc
    if not upserts:
        return stats
    docs = [d for _, ds in upserts for d in ds]
    try:
        adapter.upsert_batch(docs)
        stats["upserted"] = len(upserts)
        stats["chunks"] = len(docs)
        return stats
    except EmbeddingUnavailable as exc:
        if not CLIENT_ERROR_RE.search(str(exc)):
            raise InfraError(f"embedding: {exc}") from exc
    except Exception as exc:  # noqa: BLE001
        raise InfraError(f"vector store upsert: {exc}") from exc
    # The embedding server rejected the batch as a client error: isolate the offending rows.
    for c, ds in upserts:
        try:
            adapter.upsert_batch(ds)
            stats["upserted"] += 1
            stats["chunks"] += len(ds)
        except EmbeddingUnavailable as exc:
            if CLIENT_ERROR_RE.search(str(exc)):
                dead(c, f"embedding rejected this row: {exc}")
            else:
                raise InfraError(f"embedding: {exc}") from exc
        except Exception as exc:  # noqa: BLE001
            raise InfraError(f"vector store upsert: {exc}") from exc
    return stats


class VectorStore:
    """pydebeziumai's PGVectorAdapter plus "delete every vector of these rows"
    (langchain_postgres keeps metadata in cmetadata jsonb)."""

    def __init__(self, adapter: Any, target: dict[str, Any]) -> None:
        self.adapter = adapter
        self.target = target
        self.store = adapter.store

    def upsert_batch(self, docs: list[Any]) -> None:
        self.adapter.upsert_batch(docs)

    def upsert(self, doc: Any) -> None:
        self.adapter.upsert(doc)

    def delete_batch(self, ids: list[str]) -> None:
        self.adapter.delete_batch(ids)

    def delete_parents(self, parent_ids: list[str]) -> None:
        if not parent_ids:
            return
        import psycopg

        with psycopg.connect(**pg_conninfo(self.target)) as conn:
            conn.execute(
                "DELETE FROM langchain_pg_embedding e USING langchain_pg_collection c "
                "WHERE c.uuid = e.collection_id AND c.name = %s AND e.cmetadata->>'_parent_id' = ANY(%s)",
                (self.target["collection"], list(parent_ids)),
            )


# ── worker ───────────────────────────────────────────────────────────────────────────────

class Settings:
    def __init__(self) -> None:
        self.bootstrap = os.environ.get("VECTORSYNC_KAFKA_BOOTSTRAP", "cdc-message:9092")
        self.registry = os.environ.get("VECTORSYNC_SCHEMA_REGISTRY_URL", "http://cdc-message:8081")
        self.registry_auth = os.environ.get("VECTORSYNC_SCHEMA_REGISTRY_AUTH", "")
        self.group_prefix = os.environ.get("VECTORSYNC_GROUP_PREFIX", "cdc-vectorsync")
        self.max_backoff = float(os.environ.get("VECTORSYNC_MAX_BACKOFF_SECONDS", "60"))
        self.kafka_extra: dict[str, str] = {}
        for k, v in os.environ.items():  # VECTORSYNC_KAFKA_SECURITY_PROTOCOL → security.protocol
            if k.startswith("VECTORSYNC_KAFKA_") and k != "VECTORSYNC_KAFKA_BOOTSTRAP":
                self.kafka_extra[k[len("VECTORSYNC_KAFKA_") :].lower().replace("_", ".")] = v


class Sync:
    def __init__(self, spec: dict[str, Any], settings: Settings, state: StateFile, dlq: DeadLetters) -> None:
        self.spec = spec
        self.id = str(spec["id"])
        self.settings = settings
        self.state_file = state
        self.dlq = dlq
        self.generation = int(spec.get("generation") or 0)
        self.group_id = f"{settings.group_prefix}-{self.id}-g{self.generation}"
        self._stop = threading.Event()
        self._lock = threading.Lock()
        self.adapter: Any = None
        self.embeddings: ProviderEmbeddings | None = None
        self.status: dict[str, Any] = {
            "id": self.id,
            "name": spec.get("name") or "",
            "revision": spec.get("revision") or "",
            "generation": self.generation,
            "group_id": self.group_id,
            "state": "starting",
            "message": "Starting",
            "started_at": utc_now(),
            "last_event_at": None,
            "last_commit_at": None,
            "last_error": None,
            "last_error_at": None,
            "counters": {"events": 0, "upserted": 0, "chunks": 0, "deleted": 0, "skipped": 0, "dead_lettered": 0, "batches": 0, "retries": 0},
            "lag": {"total": None, "topics": {}},
            "documents": None,
            "retention_trimmed": [],
            "missing_topics": [],
            "embedding": {"provider": None, "model": None, "avg_ms": None, "dimensions": None},
        }
        self.recent_errors: deque[dict[str, Any]] = deque(maxlen=20)
        self.thread = threading.Thread(target=self._run, name=f"vectorsync-{self.id}", daemon=True)

    # public -------------------------------------------------------------------------
    def start(self) -> None:
        self.thread.start()

    def stop(self, wait: float = 15) -> None:
        self._stop.set()
        if self.thread.is_alive() and threading.current_thread() is not self.thread:
            self.thread.join(timeout=wait)

    def snapshot(self) -> dict[str, Any]:
        with self._lock:
            out = json.loads(json.dumps(self.status, default=str))
        out["counters"]["dead_letters_stored"] = self.dlq.count(self.id)
        out["topics"] = list(self.spec.get("topics") or [])
        out["collection"] = (self.spec.get("target") or {}).get("collection")
        out["recent_errors"] = list(self.recent_errors)
        return out

    def search(self, query: str, k: int, metadata_filter: dict[str, Any] | None) -> list[dict[str, Any]]:
        if self.adapter is None:
            raise InfraError("the sync is not connected to the vector store yet")
        return bridge.search(self.adapter, query, k, metadata_filter)

    # internals ----------------------------------------------------------------------
    def _set(self, **fields: Any) -> None:
        with self._lock:
            self.status.update(fields)

    def _count(self, **inc: int) -> None:
        with self._lock:
            for k, v in inc.items():
                self.status["counters"][k] = self.status["counters"].get(k, 0) + v

    def _error(self, state: str, msg: str) -> None:
        now = utc_now()
        self.recent_errors.appendleft({"at": now, "error": msg[:500]})
        self._set(state=state, message=msg[:300], last_error=msg[:2000], last_error_at=now)

    def _sleep(self, seconds: float) -> None:
        self._stop.wait(seconds)

    def _backoff(self, attempt: int) -> float:
        return min(self.settings.max_backoff, 2 ** min(attempt, 6)) * random.uniform(0.75, 1.25)

    def _connect_store(self) -> None:
        tgt = self.spec["target"]
        self.embeddings = ProviderEmbeddings(self.spec.get("embedding") or {})
        applied = int(self.state_file.get(self.id).get("generation", -1))
        reset = applied != self.generation and applied >= 0  # rebuild: drop the old vectors once
        self.adapter = VectorStore(bridge.make_adapter(pg_url(tgt), tgt["collection"], self.embeddings, reset=reset), tgt)
        self.state_file.set(self.id, generation=self.generation, collection=tgt["collection"], updated_at=utc_now())
        if reset:
            threading.Thread(target=delete_groups, args=(self.settings, self.id, self.generation), daemon=True).start()
        with self._lock:
            self.status["embedding"].update(self.embeddings.public())

    def _consumer(self) -> Any:
        from confluent_kafka import Consumer

        conf = {
            "bootstrap.servers": self.settings.bootstrap,
            "group.id": self.group_id,
            "client.id": f"cdc-vectorsync-{self.id}",
            "enable.auto.commit": False,
            "auto.offset.reset": "latest" if self.spec.get("start_from") == "latest" else "earliest",
            "max.poll.interval.ms": 900000,
            "session.timeout.ms": 45000,
            "allow.auto.create.topics": False,
            **self.settings.kafka_extra,
        }
        c = Consumer(conf)
        c.subscribe(list(self.spec["topics"]))
        return c

    def _run(self) -> None:
        attempt = 0
        while not self._stop.is_set():
            consumer = None
            try:
                self._set(state="connecting", message="Connecting to the vector store")
                self._connect_store()
                self._set(state="connecting", message="Joining the Kafka consumer group")
                consumer = self._consumer()
                builder = bridge.make_builder(self.spec)
                self.shaper = Shaper(self.spec)
                decoder = Decoder(self.settings.registry, self.settings.registry_auth)
                self._set(state="running", message="Waiting for changes")
                attempt = 0
                self._loop(consumer, builder, decoder)
            except InfraError as exc:
                attempt += 1
                self._count(retries=1)
                self._error("retrying", f"{exc}")
            except Exception as exc:  # noqa: BLE001 — connect/setup failures
                attempt += 1
                self._count(retries=1)
                self._error("retrying", f"{type(exc).__name__}: {exc}")
                traceback.print_exc()
            finally:
                if consumer is not None:
                    try:
                        consumer.close()
                    except Exception:  # noqa: BLE001
                        pass
            if not self._stop.is_set():
                wait = self._backoff(attempt)
                self._set(message=f"{self.status.get('message')} — retrying in {int(wait)}s")
                self._sleep(wait)
        self._set(state="stopped", message="Stopped")

    def _loop(self, consumer: Any, builder: Any, decoder: Decoder) -> None:
        from confluent_kafka import KafkaError, KafkaException, TopicPartition

        batch = self.spec.get("batch") or {}
        max_events = max(1, min(int(batch.get("max_events") or 200), 5000))
        max_wait = max(0.1, min(float(batch.get("max_wait_ms") or 1000) / 1000, 30))
        soft = (self.spec.get("delete_mode") or "hard") == "soft"
        last_meta = 0.0
        inner_attempt = 0
        while not self._stop.is_set():
            now = time.monotonic()
            if now - last_meta > 15:
                last_meta = now
                self._refresh_meta(consumer)
            msgs = consumer.consume(num_messages=max_events, timeout=max_wait)
            if not msgs:
                continue
            first: dict[tuple[str, int], int] = {}
            last: dict[tuple[str, int], int] = {}
            changes: list[bridge.Change] = []
            dead: list[dict[str, Any]] = []
            skipped = 0
            missing: set[str] = set()
            for m in msgs:
                err = m.error()
                if err is not None:
                    if err.code() == KafkaError._PARTITION_EOF:
                        continue
                    if err.code() in (KafkaError.UNKNOWN_TOPIC_OR_PART, KafkaError._UNKNOWN_TOPIC):
                        missing.add(m.topic() or "")
                        continue
                    raise InfraError(f"kafka: {err.str()}")
                tp = (m.topic(), m.partition())
                first.setdefault(tp, m.offset())
                last[tp] = m.offset()
                try:
                    key = decoder.decode(m.key())
                    value = decoder.decode(m.value())
                    ev = bridge.to_event(m.topic(), m.partition(), key, value)
                    if ev is None:
                        skipped += 1
                        continue
                    did = bridge.doc_id(builder, ev)
                    changes.append(bridge.Change(ev, did, ev.payload.op, m.topic(), m.partition(), m.offset()))
                except RegistryUnavailable as exc:
                    self._rewind(consumer, first, TopicPartition)
                    raise InfraError(str(exc)) from exc
                except (DecodeError, bridge.DataError) as exc:
                    dead.append(self._dead_item(m.topic(), m.partition(), m.offset(), _short_key(m.key()), None, str(exc)))
            if missing:
                self._set(missing_topics=sorted(missing))

            def to_dead(c: bridge.Change, msg: str) -> None:
                dead.append(self._dead_item(c.topic, c.partition, c.offset, c.event.key, c.doc_id, msg, c.op))

            try:
                stats = apply_batch(changes, builder, self.adapter, soft, to_dead, self.shaper)
            except InfraError as exc:
                inner_attempt += 1
                self._count(retries=1)
                self._error("retrying", str(exc))
                self._rewind(consumer, first, TopicPartition)
                self._sleep(self._backoff(inner_attempt))
                continue
            inner_attempt = 0
            self.dlq.add(self.id, dead)
            if last:
                try:
                    consumer.commit(offsets=[TopicPartition(t, p, o + 1) for (t, p), o in last.items()], asynchronous=False)
                except KafkaException as exc:
                    # A rebalance moved the partition; the new owner re-reads (writes are idempotent).
                    self._error("retrying", f"commit: {exc}")
                    continue
            self._count(events=len(msgs), upserted=stats["upserted"], chunks=stats.get("chunks", 0), deleted=stats["deleted"], skipped=skipped, dead_lettered=len(dead), batches=1)
            with self._lock:
                self.status["state"] = "running"
                self.status["message"] = "In sync" if not dead else f"{len(dead)} row(s) dead-lettered in the last batch"
                self.status["last_event_at"] = utc_now()
                self.status["last_commit_at"] = utc_now()
                if self.embeddings is not None:
                    self.status["embedding"]["avg_ms"] = self.embeddings.avg_ms

    def _rewind(self, consumer: Any, first: dict[tuple[str, int], int], tp_cls: Any) -> None:
        for (t, p), off in first.items():
            try:
                consumer.seek(tp_cls(t, p, off))
            except Exception:  # noqa: BLE001 — not assigned any more: the next owner re-reads
                pass

    def _dead_item(self, topic: str, partition: int, offset: int, key: Any, doc_id: str | None, error: str, op: str | None = None) -> dict[str, Any]:
        return {"at": utc_now(), "topic": topic, "partition": partition, "offset": offset, "key": (str(key)[:300] if key is not None else None), "doc_id": doc_id, "op": op, "error": error[:1000]}

    def _refresh_meta(self, consumer: Any) -> None:
        """Lag per topic, retention trimming, document count. Best effort; never raises."""
        try:
            assignment = consumer.assignment()
            per_topic: dict[str, int] = {}
            trimmed: set[str] = set()
            if assignment:
                committed = {(tp.topic, tp.partition): tp.offset for tp in consumer.committed(assignment, timeout=5)}
                for tp in assignment:
                    low, high = consumer.get_watermark_offsets(tp, timeout=5, cached=False)
                    pos = committed.get((tp.topic, tp.partition), -1001)
                    if pos < 0:  # nothing committed yet: everything from the start offset is pending
                        pos = high if self.spec.get("start_from") == "latest" else low
                        if low > 0 and self.spec.get("start_from") != "latest":
                            trimmed.add(tp.topic)
                    per_topic[tp.topic] = per_topic.get(tp.topic, 0) + max(0, high - pos)
            with self._lock:
                self.status["lag"] = {"total": sum(per_topic.values()) if assignment else None, "topics": per_topic}
                if trimmed:
                    self.status["retention_trimmed"] = sorted(set(self.status["retention_trimmed"]) | trimmed)
        except Exception:  # noqa: BLE001
            pass
        try:
            import psycopg

            with psycopg.connect(**pg_conninfo(self.spec["target"])) as conn:
                row = conn.execute(
                    "SELECT count(*) FROM langchain_pg_embedding e JOIN langchain_pg_collection c ON c.uuid = e.collection_id WHERE c.name = %s",
                    (self.spec["target"]["collection"],),
                ).fetchone()
            self._set(documents=int(row[0]) if row else 0)
        except Exception:  # noqa: BLE001
            pass


def drop_collection(target: dict[str, Any]) -> int:
    """Delete one collection and its vectors (langchain_pg_embedding cascades). Fresh connection,
    so it works whether or not the sync is running. Returns the number of documents removed."""
    import psycopg

    coll = str(target.get("collection") or "")
    if not COLLECTION_RE.match(coll):
        raise BadSpec("target.collection is not a collection name")
    with psycopg.connect(**pg_conninfo(target)) as conn:
        if not conn.execute("SELECT to_regclass('langchain_pg_collection') IS NOT NULL").fetchone()[0]:
            return 0
        row = conn.execute(
            "SELECT count(e.*) FROM langchain_pg_collection c LEFT JOIN langchain_pg_embedding e ON e.collection_id = c.uuid WHERE c.name = %s",
            (coll,),
        ).fetchone()
        conn.execute("DELETE FROM langchain_pg_collection WHERE name = %s", (coll,))
    return int(row[0]) if row else 0


def delete_groups(settings: Settings, sync_id: str, keep_generation: int | None) -> list[str]:
    """Remove this sync's consumer groups (all, or all but the current generation). Best effort."""
    try:
        from confluent_kafka.admin import AdminClient

        admin = AdminClient({"bootstrap.servers": settings.bootstrap, **settings.kafka_extra})
        prefix = f"{settings.group_prefix}-{sync_id}-g"
        listed = admin.list_consumer_groups(request_timeout=10).result(timeout=15)
        names = [g.group_id for g in listed.valid if g.group_id.startswith(prefix)]
        if keep_generation is not None:
            names = [n for n in names if n != f"{prefix}{keep_generation}"]
        if not names:
            return []
        for fut in admin.delete_consumer_groups(names, request_timeout=10).values():
            try:
                fut.result(timeout=15)
            except Exception:  # noqa: BLE001 — group still has members (rebalance in progress)
                pass
        return names
    except Exception:  # noqa: BLE001
        return []


def _short_key(raw: bytes | None) -> str | None:
    if raw is None:
        return None
    return raw[:120].hex() if raw[:1] == b"\x00" else raw[:300].decode("utf-8", "replace")


class Registry:
    """Running syncs by id. PUT with an unchanged revision+generation is a no-op."""

    def __init__(self, settings: Settings, state: StateFile, dlq: DeadLetters) -> None:
        self.settings = settings
        self.state = state
        self.dlq = dlq
        self._syncs: dict[str, Sync] = {}
        self._lock = threading.Lock()

    def put(self, spec: dict[str, Any]) -> dict[str, Any]:
        validate_spec(spec)
        sid = str(spec["id"])
        with self._lock:
            cur = self._syncs.get(sid)
            if cur and cur.spec.get("revision") == spec.get("revision") and cur.generation == int(spec.get("generation") or 0):
                return cur.snapshot()
            if cur:
                cur.stop()
            s = Sync(spec, self.settings, self.state, self.dlq)
            self._syncs[sid] = s
            s.start()
            return s.snapshot()

    def get(self, sid: str) -> Sync | None:
        with self._lock:
            return self._syncs.get(sid)

    def list(self) -> list[dict[str, Any]]:
        with self._lock:
            syncs = list(self._syncs.values())
        return [s.snapshot() for s in syncs]

    def delete(self, sid: str, forget: bool = False) -> bool:
        with self._lock:
            s = self._syncs.pop(sid, None)
        if s:
            s.stop()
        if forget:
            self.state.drop(sid)
            self.dlq.clear(sid)
            delete_groups(self.settings, sid, None)
        return s is not None

    def stop_all(self) -> None:
        with self._lock:
            syncs = list(self._syncs.values())
            self._syncs.clear()
        for s in syncs:
            s.stop(wait=5)
