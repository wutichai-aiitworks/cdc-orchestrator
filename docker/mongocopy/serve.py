#!/usr/bin/env python3
"""HTTP sidecar to run mongodump + mongorestore (Mongo → Mongo)."""

from __future__ import annotations

import json
import os
import signal
import subprocess
import threading
import uuid
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import quote, urlparse

WORK_ROOT = Path(os.environ.get("MONGOCOPY_WORK_ROOT", "/var/lib/mongocopy"))
JOBS_DIR = WORK_ROOT / "jobs"
API_TOKEN = os.environ.get("MONGOCOPY_API_TOKEN", "").strip()
HOST = os.environ.get("MONGOCOPY_API_HOST", "0.0.0.0")
PORT = int(os.environ.get("MONGOCOPY_API_PORT", "8088"))

_lock = threading.Lock()
_procs: dict[str, subprocess.Popen[Any]] = {}


def utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def job_dir(job_id: str) -> Path:
    return JOBS_DIR / job_id


def read_json(path: Path) -> dict[str, Any]:
    if not path.is_file():
        return {}
    with path.open("r", encoding="utf-8") as fh:
        return json.load(fh)


def write_json(path: Path, data: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    with tmp.open("w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2)
    tmp.replace(path)


def mongo_uri(conn: dict[str, Any], options: dict[str, Any]) -> str:
    user = quote(str(conn.get("username") or conn.get("user") or ""), safe="")
    password = quote(str(conn.get("password") or ""), safe="")
    host = conn.get("host") or "localhost"
    port = str(conn.get("port") or "27017")
    database = str(conn.get("database") or "")
    auth_source = str(options.get("auth_source") or conn.get("schema") or "").strip()
    auth = ""
    if user and password:
        auth = f"{user}:{password}@"
    elif user:
        auth = f"{user}@"
    path = f"/{database}" if database else "/"
    query = f"?authSource={quote(auth_source)}" if auth_source else ""
    return f"mongodb://{auth}{host}:{port}{path}{query}"


def ns_filters(body: dict[str, Any]) -> list[str]:
    source = body.get("source") or {}
    db = str(source.get("database") or "").strip()
    items = [str(item).strip() for item in (body.get("include_collections") or []) if str(item).strip()]
    out: list[str] = []
    for item in items:
        if "." in item:
            out.append(item)
        elif db:
            out.append(f"{db}.{item}")
    return out


def write_run_script(work: Path, body: dict[str, Any]) -> Path:
    source = body.get("source") or {}
    target = body.get("target") or {}
    options = body.get("options") or {}
    src_uri = mongo_uri(source, options)
    tgt_uri = mongo_uri(target, options)
    archive = work / "dump.archive.gz"
    gzip = options.get("gzip", True)
    drop = bool(options.get("drop"))
    filters = ns_filters(body)
    src_db = str(source.get("database") or "").strip()
    tgt_db = str(target.get("database") or "").strip()

    dump = ["mongodump", f"--uri={src_uri}", f"--archive={archive}"]
    restore = ["mongorestore", f"--uri={tgt_uri}", f"--archive={archive}"]
    if gzip:
        dump.append("--gzip")
        restore.append("--gzip")
    if drop:
        restore.append("--drop")
    for ns in filters:
        dump.extend(["--nsInclude", ns])
        restore.extend(["--nsInclude", ns])
    if src_db and tgt_db and src_db != tgt_db and not filters:
        restore.extend([f"--nsFrom={src_db}.*", f"--nsTo={tgt_db}.*"])

    def sh_join(parts: list[str]) -> str:
        return " ".join("'" + p.replace("'", "'\\''") + "'" for p in parts)

    script = work / "run.sh"
    script.write_text(
        "#!/bin/sh\nset -eu\n" + sh_join(dump) + "\n" + sh_join(restore) + "\n",
        encoding="utf-8",
    )
    script.chmod(0o755)
    return script


def job_status(job_id: str) -> dict[str, Any]:
    meta_path = job_dir(job_id) / "job.json"
    meta = read_json(meta_path)
    if not meta:
        return {"id": job_id, "status": "unknown"}
    with _lock:
        proc = _procs.get(job_id)
    if proc is not None:
        code = proc.poll()
        if code is None:
            meta["status"] = "running"
        else:
            meta["status"] = "succeeded" if code == 0 else "failed"
            meta["exit_code"] = code
            meta["finished_at"] = utc_now()
            write_json(meta_path, meta)
            _procs.pop(job_id, None)
    elif meta.get("status") == "running":
        meta["status"] = "failed"
        meta["error"] = meta.get("error") or "process not found"
        meta["finished_at"] = utc_now()
        write_json(meta_path, meta)
    meta["id"] = job_id
    return meta


def tail_log(path: Path, max_bytes: int = 256_000) -> str:
    if not path.is_file():
        return ""
    data = path.read_bytes()
    if len(data) > max_bytes:
        data = data[-max_bytes:]
    return data.decode("utf-8", errors="replace")


def start_job(body: dict[str, Any]) -> dict[str, Any]:
    job_id = str(body.get("id") or uuid.uuid4())
    work = job_dir(job_id)
    work.mkdir(parents=True, exist_ok=True)
    script = write_run_script(work, body)
    argv = ["/bin/sh", str(script)]
    log_path = work / "copy.log"
    meta = {
        "id": job_id,
        "name": body.get("name") or job_id,
        "status": "starting",
        "created_at": utc_now(),
        "started_at": utc_now(),
        "argv": ["mongodump", "&&", "mongorestore"],
        "log_path": str(log_path),
    }
    write_json(work / "job.json", meta)
    with log_path.open("ab") as log_fh:
        log_fh.write(f"\n--- mongocopy job {job_id} at {utc_now()} ---\n".encode())
        proc = subprocess.Popen(
            argv,
            stdout=log_fh,
            stderr=subprocess.STDOUT,
            cwd=str(work),
            start_new_session=True,
        )
    with _lock:
        _procs[job_id] = proc
    meta["status"] = "running"
    meta["pid"] = proc.pid
    write_json(work / "job.json", meta)
    return meta


def stop_job(job_id: str) -> dict[str, Any]:
    with _lock:
        proc = _procs.get(job_id)
    if proc is None or proc.poll() is not None:
        meta = job_status(job_id)
        if meta.get("status") == "running":
            meta["status"] = "stopped"
            meta["finished_at"] = utc_now()
            write_json(job_dir(job_id) / "job.json", meta)
        return meta
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    meta = job_status(job_id)
    meta["status"] = "stopped"
    meta["finished_at"] = utc_now()
    write_json(job_dir(job_id) / "job.json", meta)
    with _lock:
        _procs.pop(job_id, None)
    return meta


def list_jobs() -> list[dict[str, Any]]:
    if not JOBS_DIR.is_dir():
        return []
    return [
        job_status(child.name)
        for child in sorted(JOBS_DIR.iterdir(), key=lambda p: p.name, reverse=True)
        if child.is_dir()
    ]


CAPABILITY = {
    "service": "cdc-mongocopy",
    "command": "mongodump && mongorestore",
    "source_types": ["mongo", "mongodb"],
    "target_types": ["mongo", "mongodb"],
    "options": {
        "booleans": ["drop", "gzip"],
        "strings": ["auth_source"],
    },
    "connection_fields": ["host", "port", "database", "username", "password"],
    "include_collections": "Optional db.collection or collection names; empty means whole source database",
}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt: str, *args: Any) -> None:
        return

    def _auth_ok(self) -> bool:
        if not API_TOKEN:
            return True
        auth = self.headers.get("Authorization", "")
        if auth.startswith("Bearer ") and auth[7:].strip() == API_TOKEN:
            return True
        return self.headers.get("X-API-Token", "").strip() == API_TOKEN

    def _json(self, code: int, payload: Any) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _read_body(self) -> dict[str, Any]:
        length = int(self.headers.get("Content-Length", "0") or "0")
        raw = self.rfile.read(length) if length else b"{}"
        try:
            return json.loads(raw.decode("utf-8"))
        except json.JSONDecodeError:
            return {}

    def do_GET(self) -> None:
        if not self._auth_ok():
            self._json(401, {"error": "unauthorized"})
            return
        path = urlparse(self.path).path.rstrip("/") or "/"
        if path == "/health":
            self._json(200, {"status": "ok"})
            return
        if path == "/v1/discover":
            self._json(
                200,
                {
                    "contract_version": "1",
                    "tool_id": "mongocopy",
                    "service": CAPABILITY["service"],
                    "sidecar_ready": True,
                    "orchestrator_contract": "/cdc/mongocopy/contract",
                    "options": CAPABILITY["options"],
                },
            )
            return
        if path == "/v1/capability":
            self._json(200, CAPABILITY)
            return
        if path == "/v1/jobs":
            self._json(200, {"jobs": list_jobs()})
            return
        if path.startswith("/v1/jobs/") and path.endswith("/logs"):
            job_id = path.split("/")[3]
            self._json(200, {"id": job_id, "log": tail_log(job_dir(job_id) / "copy.log")})
            return
        if path.startswith("/v1/jobs/"):
            job_id = path.split("/")[-1]
            meta = job_status(job_id)
            if meta.get("status") == "unknown":
                self._json(404, {"error": "job not found"})
                return
            self._json(200, meta)
            return
        self._json(404, {"error": "not found"})

    def do_POST(self) -> None:
        if not self._auth_ok():
            self._json(401, {"error": "unauthorized"})
            return
        path = urlparse(self.path).path.rstrip("/")
        if path == "/v1/jobs":
            body = self._read_body()
            if not body.get("source") or not body.get("target"):
                self._json(400, {"error": "source and target are required"})
                return
            try:
                meta = start_job(body)
            except OSError as exc:
                self._json(500, {"error": str(exc)})
                return
            self._json(202, meta)
            return
        if path.startswith("/v1/jobs/") and path.endswith("/stop"):
            self._json(200, stop_job(path.split("/")[3]))
            return
        self._json(404, {"error": "not found"})

    def do_DELETE(self) -> None:
        if not self._auth_ok():
            self._json(401, {"error": "unauthorized"})
            return
        path = urlparse(self.path).path.rstrip("/")
        if path.startswith("/v1/jobs/"):
            job_id = path.split("/")[-1]
            stop_job(job_id)
            self._json(200, {"id": job_id, "status": "stopped"})
            return
        self._json(404, {"error": "not found"})


def main() -> None:
    JOBS_DIR.mkdir(parents=True, exist_ok=True)
    server = ThreadingHTTPServer((HOST, PORT), Handler)
    print(f"cdc-mongocopy listening on {HOST}:{PORT}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
