#!/usr/bin/env python3
"""HTTP sidecar to run pgloader jobs (MySQL/SQL Server/SQLite/CSV → Postgres)."""

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

WORK_ROOT = Path(os.environ.get("PGLOADER_WORK_ROOT", "/var/lib/pgloader"))
JOBS_DIR = WORK_ROOT / "jobs"
API_TOKEN = os.environ.get("PGLOADER_API_TOKEN", "").strip()
HOST = os.environ.get("PGLOADER_API_HOST", "0.0.0.0")
PORT = int(os.environ.get("PGLOADER_API_PORT", "8087"))

SOURCE_ALIASES = {
    "mysql": "mysql",
    "mariadb": "mysql",
    "mssql": "mssql",
    "sqlserver": "mssql",
    "sqlite": "sqlite",
    "csv": "csv",
}

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


def conn_type(conn: dict[str, Any]) -> str:
    raw = str(conn.get("type") or conn.get("db_type") or "").strip().lower()
    return SOURCE_ALIASES.get(raw, raw)


def quote_ident(value: str) -> str:
    return "'" + str(value).replace("'", "''") + "'"


def sql_uri(scheme: str, conn: dict[str, Any], default_port: str) -> str:
    user = quote(str(conn.get("username") or conn.get("user") or ""), safe="")
    password = quote(str(conn.get("password") or ""), safe="")
    host = conn.get("host") or "localhost"
    port = str(conn.get("port") or default_port)
    database = str(conn.get("database") or "")
    auth = ""
    if user and password:
        auth = f"{user}:{password}@"
    elif user:
        auth = f"{user}@"
    path = f"/{database}" if database else ""
    return f"{scheme}://{auth}{host}:{port}{path}"


def file_path(conn: dict[str, Any]) -> str:
    return str(conn.get("database") or conn.get("path") or conn.get("host") or "").strip()


def write_load_file(work: Path, body: dict[str, Any]) -> Path:
    source = body.get("source") or {}
    target = body.get("target") or {}
    options = body.get("options") or {}
    include_tables = [str(item).strip() for item in (body.get("include_tables") or []) if str(item).strip()]
    src_type = conn_type(source)
    if src_type not in SOURCE_ALIASES.values():
        raise ValueError(f"unsupported pgloader source type: {src_type or '(empty)'}")

    target_uri = sql_uri("postgresql", target, "5432")
    with_parts: list[str] = []
    with_parts.append("include drop" if options.get("drop_if_exists") else "include no drop")
    with_parts.append("create tables" if options.get("create_tables", True) else "create no tables")
    with_parts.append("create indexes" if options.get("create_indexes", True) else "create no indexes")
    with_parts.append("reset sequences" if options.get("reset_sequences", True) else "reset no sequences")
    if options.get("truncate"):
        with_parts.append("truncate")
    for key, flag in (("workers", "workers"), ("concurrency", "concurrency")):
        val = options.get(key)
        if val is not None and str(val).strip() != "":
            with_parts.append(f"{flag} = {int(val)}")

    lines: list[str]
    if src_type == "csv":
        path = file_path(source)
        table = str(options.get("target_table") or "").strip()
        if not path or not table:
            raise ValueError("CSV source needs a file path (database) and options.target_table")
        delim = str(options.get("csv_delimiter") or ",").replace("'", "''")
        skip = int(options.get("csv_skip_header") or 1)
        lines = [
            "LOAD CSV",
            f"     FROM {quote_ident(path)}",
            f"     INTO {target_uri}",
            f"     TARGET TABLE {table}",
            f"     WITH fields terminated by '{delim}', skip header = {skip}",
            ";",
        ]
    elif src_type == "sqlite":
        path = file_path(source)
        if not path:
            raise ValueError("SQLite source needs a file path in database")
        uri = path if path.startswith("sqlite://") else f"sqlite://{path}"
        lines = [
            "LOAD DATABASE",
            f"     FROM {uri}",
            f"     INTO {target_uri}",
            f"     WITH {', '.join(with_parts)}",
        ]
        if include_tables:
            matched = ", ".join(quote_ident(item) for item in include_tables)
            lines.append(f"     INCLUDING ONLY TABLE NAMES MATCHING {matched}")
        lines.append(";")
    else:
        scheme = "mysql" if src_type == "mysql" else "mssql"
        default_port = "3306" if src_type == "mysql" else "1433"
        lines = [
            "LOAD DATABASE",
            f"     FROM {sql_uri(scheme, source, default_port)}",
            f"     INTO {target_uri}",
            f"     WITH {', '.join(with_parts)}",
        ]
        if include_tables:
            matched = ", ".join(quote_ident(item) for item in include_tables)
            lines.append(f"     INCLUDING ONLY TABLE NAMES MATCHING {matched}")
        lines.append(";")

    path = work / "load.load"
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return path


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
    load_path = write_load_file(work, body)
    argv = ["pgloader", str(load_path)]
    log_path = work / "load.log"
    meta = {
        "id": job_id,
        "name": body.get("name") or job_id,
        "status": "starting",
        "created_at": utc_now(),
        "started_at": utc_now(),
        "argv": ["pgloader", "load.load"],
        "log_path": str(log_path),
    }
    write_json(work / "job.json", meta)

    with log_path.open("ab") as log_fh:
        log_fh.write(f"\n--- pgloader job {job_id} at {utc_now()} ---\n".encode())
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
    return [job_status(child.name) for child in sorted(JOBS_DIR.iterdir(), key=lambda p: p.name, reverse=True) if child.is_dir()]


CAPABILITY = {
    "service": "cdc-pgloader",
    "command": "pgloader",
    "source_types": ["mysql", "mariadb", "mssql", "sqlserver", "sqlite", "csv"],
    "target_types": ["postgres", "postgresql"],
    "options": {
        "booleans": ["drop_if_exists", "create_tables", "create_indexes", "reset_sequences", "truncate"],
        "integers": ["workers", "concurrency", "csv_skip_header"],
        "strings": ["target_table", "csv_delimiter"],
    },
    "connection_fields": ["type", "host", "port", "database", "username", "password"],
    "include_tables": "Optional table names; empty means all tables. CSV uses options.target_table.",
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
            try:
                ver = subprocess.check_output(["pgloader", "--version"], text=True).strip().splitlines()[0]
            except (subprocess.CalledProcessError, FileNotFoundError):
                ver = "unknown"
            self._json(
                200,
                {
                    "contract_version": "1",
                    "tool_id": "pgloader",
                    "service": CAPABILITY["service"],
                    "sidecar_ready": True,
                    "orchestrator_contract": "/cdc/pgloader/contract",
                    "pgloader_version": ver,
                    "options": CAPABILITY["options"],
                },
            )
            return
        if path == "/v1/capability":
            out = dict(CAPABILITY)
            try:
                out["pgloader_version"] = subprocess.check_output(
                    ["pgloader", "--version"], text=True
                ).strip().splitlines()[0]
            except (subprocess.CalledProcessError, FileNotFoundError):
                out["pgloader_version"] = "unknown"
            self._json(200, out)
            return
        if path == "/v1/jobs":
            self._json(200, {"jobs": list_jobs()})
            return
        if path.startswith("/v1/jobs/") and path.endswith("/logs"):
            job_id = path.split("/")[3]
            self._json(200, {"id": job_id, "log": tail_log(job_dir(job_id) / "load.log")})
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
            except ValueError as exc:
                self._json(400, {"error": str(exc)})
                return
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
    print(f"cdc-pgloader listening on {HOST}:{PORT}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
