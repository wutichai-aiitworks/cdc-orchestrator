#!/usr/bin/env python3
"""Minimal HTTP sidecar to run pgcopydb clone jobs from JSON configuration."""

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
from urllib.parse import urlparse

WORK_ROOT = Path(os.environ.get("PGCOPY_WORK_ROOT", "/var/lib/pgcopydb"))
JOBS_DIR = WORK_ROOT / "jobs"
API_TOKEN = os.environ.get("PGCOPY_API_TOKEN", "").strip()
HOST = os.environ.get("PGCOPY_API_HOST", "0.0.0.0")
PORT = int(os.environ.get("PGCOPY_API_PORT", "8085"))

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


def pg_uri(conn: dict[str, Any]) -> str:
    user = conn.get("username") or conn.get("user") or ""
    password = conn.get("password") or ""
    host = conn.get("host") or "localhost"
    port = conn.get("port") or "5432"
    database = conn.get("database") or "postgres"
    if password:
        return f"postgresql://{user}:{password}@{host}:{port}/{database}"
    if user:
        return f"postgresql://{user}@{host}:{port}/{database}"
    return f"postgresql://{host}:{port}/{database}"


def write_filters(work: Path, include_tables: list[str]) -> Path | None:
    if not include_tables:
        return None
    path = work / "filters.ini"
    lines = ["[include]"]
    for item in include_tables:
        item = str(item).strip()
        if item:
            lines.append(item)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return path


BOOL_FLAGS: dict[str, str] = {
    "drop_if_exists": "--drop-if-exists",
    "roles": "--roles",
    "no_role_passwords": "--no-role-passwords",
    "no_owner": "--no-owner",
    "no_acl": "--no-acl",
    "no_comments": "--no-comments",
    "no_tablespaces": "--no-tablespaces",
    "skip_large_objects": "--skip-large-objects",
    "skip_extensions": "--skip-extensions",
    "skip_ext_comments": "--skip-ext-comments",
    "skip_collations": "--skip-collations",
    "skip_vacuum": "--skip-vacuum",
    "skip_analyze": "--skip-analyze",
    "skip_db_properties": "--skip-db-properties",
    "skip_split_by_ctid": "--skip-split-by-ctid",
    "fail_fast": "--fail-fast",
    "restart": "--restart",
    "resume": "--resume",
    "not_consistent": "--not-consistent",
    "follow": "--follow",
    "create_slot": "--create-slot",
    "wal2json_numeric_as_string": "--wal2json-numeric-as-string",
    "use_copy_binary": "--use-copy-binary",
    "all_databases": "--all-databases",
    "estimate_table_sizes": "--estimate-table-sizes",
}

INT_OPTS: dict[str, str] = {
    "table_jobs": "--table-jobs",
    "index_jobs": "--index-jobs",
    "restore_jobs": "--restore-jobs",
    "large_objects_jobs": "--large-objects-jobs",
    "split_tables_larger_than": "--split-tables-larger-than",
    "split_max_parts": "--split-max-parts",
}

STR_OPTS: dict[str, str] = {
    "plugin": "--plugin",
    "slot_name": "--slot-name",
    "origin": "--origin",
    "endpos": "--endpos",
    "snapshot": "--snapshot",
    "requirements": "--requirements",
}


def build_clone_argv(body: dict[str, Any], work: Path) -> list[str]:
    source = body.get("source") or {}
    target = body.get("target") or {}
    options = body.get("options") or {}
    include_tables = body.get("include_tables") or []

    argv = [
        "pgcopydb",
        "clone",
        "--source",
        pg_uri(source),
        "--target",
        pg_uri(target),
        "--dir",
        str(work / "clone"),
    ]

    filters = write_filters(work, include_tables)
    if filters:
        argv.extend(["--filters", str(filters)])

    for key, flag in BOOL_FLAGS.items():
        if options.get(key):
            argv.append(flag)

    for key, flag in INT_OPTS.items():
        val = options.get(key)
        if val is not None and str(val).strip() != "":
            argv.extend([flag, str(val)])

    for key, flag in STR_OPTS.items():
        val = options.get(key)
        if val is not None and str(val).strip() != "":
            argv.extend([flag, str(val)])

    extra_filters = options.get("filters_file")
    if extra_filters and str(extra_filters).strip():
        argv.extend(["--filters", str(extra_filters)])

    return argv


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

    argv = build_clone_argv(body, work)
    log_path = work / "clone.log"
    meta = {
        "id": job_id,
        "name": body.get("name") or job_id,
        "status": "starting",
        "created_at": utc_now(),
        "started_at": utc_now(),
        "argv": argv,
        "log_path": str(log_path),
    }
    write_json(work / "job.json", meta)

    with log_path.open("ab") as log_fh:
        log_fh.write(f"\n--- pgcopydb job {job_id} at {utc_now()} ---\n".encode())
        log_fh.write((" ".join(argv) + "\n").encode())
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
    jobs: list[dict[str, Any]] = []
    for child in sorted(JOBS_DIR.iterdir(), key=lambda p: p.name, reverse=True):
        if child.is_dir():
            jobs.append(job_status(child.name))
    return jobs


CAPABILITY = {
    "service": "cdc-pgcopydb",
    "pgcopydb_command": "clone",
    "options": {
        "booleans": sorted(BOOL_FLAGS.keys()),
        "integers": sorted(INT_OPTS.keys()),
        "strings": sorted(STR_OPTS.keys()),
    },
    "connection_fields": ["host", "port", "database", "username", "password"],
    "include_tables": "Optional list of schema.table entries; empty means entire database",
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
                ver = subprocess.check_output(["pgcopydb", "--version"], text=True).strip()
            except subprocess.CalledProcessError:
                ver = "unknown"
            self._json(
                200,
                {
                    "contract_version": "1",
                    "tool_id": "pgcopy",
                    "service": CAPABILITY["service"],
                    "sidecar_ready": True,
                    "orchestrator_contract": "/cdc/pgcopy/contract",
                    "pgcopydb_version": ver,
                    "options": CAPABILITY["options"],
                },
            )
            return
        if path == "/v1/capability":
            try:
                ver = subprocess.check_output(["pgcopydb", "--version"], text=True).strip()
            except subprocess.CalledProcessError:
                ver = "unknown"
            out = dict(CAPABILITY)
            out["pgcopydb_version"] = ver
            self._json(200, out)
            return
        if path == "/v1/jobs":
            self._json(200, {"jobs": list_jobs()})
            return
        if path.startswith("/v1/jobs/") and path.endswith("/logs"):
            job_id = path.split("/")[3]
            log_path = job_dir(job_id) / "clone.log"
            self._json(200, {"id": job_id, "log": tail_log(log_path)})
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
            job_id = path.split("/")[3]
            self._json(200, stop_job(job_id))
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
    print(f"cdc-pgcopydb listening on {HOST}:{PORT}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
