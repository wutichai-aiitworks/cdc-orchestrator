#!/usr/bin/env python3
"""HTTP sidecar to run RedisShake sync jobs from JSON configuration."""

from __future__ import annotations

import json
import os
import shutil
import signal
import subprocess
import threading
import uuid
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

WORK_ROOT = Path(os.environ.get("REDIS_SHAKE_WORK_ROOT", "/var/lib/redisshake"))
JOBS_DIR = WORK_ROOT / "jobs"
API_TOKEN = os.environ.get("REDIS_SHAKE_API_TOKEN", "").strip()
HOST = os.environ.get("REDIS_SHAKE_API_HOST", "0.0.0.0")
PORT = int(os.environ.get("REDIS_SHAKE_API_PORT", "8086"))
SHAKE_BIN = os.environ.get("REDIS_SHAKE_BIN", "/app/redis-shake")
MAX_CONCURRENT_JOBS = max(1, int(os.environ.get("REDIS_SHAKE_MAX_CONCURRENT_JOBS", "1")))
RETAIN_TERMINAL_JOBS = max(1, int(os.environ.get("REDIS_SHAKE_RETAIN_JOBS", "15")))
CLEANUP_STAGING = os.environ.get("REDIS_SHAKE_CLEANUP_STAGING", "true").strip().lower() in (
    "1",
    "true",
    "yes",
    "on",
)

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


def toml_str(value: str) -> str:
    escaped = str(value).replace("\\", "\\\\").replace('"', '\\"')
    return f'"{escaped}"'


def toml_bool(value: Any, default: bool = False) -> str:
    if value is None:
        return "true" if default else "false"
    return "true" if bool(value) else "false"


def redis_address(conn: dict[str, Any]) -> str:
    host = (conn.get("host") or "127.0.0.1").strip()
    port = str(conn.get("port") or "6379").strip()
    return f"{host}:{port}"


def side_options(options: dict[str, Any], prefix: str) -> tuple[bool, bool]:
    cluster = bool(options.get(f"{prefix}_cluster"))
    tls = bool(options.get(f"{prefix}_tls"))
    return cluster, tls


def write_shake_toml(work: Path, body: dict[str, Any]) -> Path:
    source = body.get("source") or {}
    target = body.get("target") or {}
    options = body.get("options") or {}

    src_cluster, src_tls = side_options(options, "source")
    tgt_cluster, tgt_tls = side_options(options, "target")

    lines = [
        "[sync_reader]",
        f"cluster = {toml_bool(src_cluster)}",
        f"address = {toml_str(redis_address(source))}",
        f"username = {toml_str(source.get('username') or '')}",
        f"password = {toml_str(source.get('password') or '')}",
        f"tls = {toml_bool(src_tls)}",
        f"sync_rdb = {toml_bool(options.get('sync_rdb'), True)}",
        f"sync_aof = {toml_bool(options.get('sync_aof'), True)}",
        f"prefer_replica = {toml_bool(options.get('prefer_replica'))}",
        f"try_diskless = {toml_bool(options.get('try_diskless'))}",
        "",
        "[redis_writer]",
        f"cluster = {toml_bool(tgt_cluster)}",
        f"address = {toml_str(redis_address(target))}",
        f"username = {toml_str(target.get('username') or '')}",
        f"password = {toml_str(target.get('password') or '')}",
        f"tls = {toml_bool(tgt_tls)}",
        f"off_reply = {toml_bool(options.get('off_reply'))}",
        "",
    ]

    behavior = (options.get("rdb_restore_command_behavior") or "panic").strip()
    if behavior in ("panic", "rewrite", "skip"):
        lines.extend(
            [
                "[advanced]",
                f"rdb_restore_command_behavior = {toml_str(behavior)}",
                "",
            ]
        )

    path = work / "shake.toml"
    path.write_text("\n".join(lines), encoding="utf-8")
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
            cleanup_staging(job_id)
            _procs.pop(job_id, None)
    elif meta.get("status") == "running":
        meta["status"] = "failed"
        meta["error"] = meta.get("error") or "process not found"
        meta["finished_at"] = utc_now()
        write_json(meta_path, meta)
        cleanup_staging(job_id)

    meta["id"] = job_id
    return meta


def running_job_count() -> int:
    count = 0
    if not JOBS_DIR.is_dir():
        return 0
    for child in JOBS_DIR.iterdir():
        if child.is_dir() and job_status(child.name).get("status") == "running":
            count += 1
    return count


def cleanup_staging(job_id: str) -> None:
    if not CLEANUP_STAGING:
        return
    data = job_dir(job_id) / "data"
    if data.is_dir():
        shutil.rmtree(data, ignore_errors=True)


def prune_terminal_jobs() -> None:
    if not JOBS_DIR.is_dir():
        return
    terminal: list[tuple[float, Path]] = []
    for child in JOBS_DIR.iterdir():
        if not child.is_dir():
            continue
        meta = read_json(child / "job.json")
        status = meta.get("status")
        if status in ("running", "starting"):
            continue
        if status in ("succeeded", "failed", "stopped", "unknown"):
            terminal.append((child.stat().st_mtime, child))
    terminal.sort(key=lambda item: item[0], reverse=True)
    for _, path in terminal[RETAIN_TERMINAL_JOBS:]:
        shutil.rmtree(path, ignore_errors=True)


class JobBusyError(Exception):
    pass


def tail_log(path: Path, max_bytes: int = 256_000) -> str:
    if not path.is_file():
        return ""
    data = path.read_bytes()
    if len(data) > max_bytes:
        data = data[-max_bytes:]
    return data.decode("utf-8", errors="replace")


def start_job(body: dict[str, Any]) -> dict[str, Any]:
    if running_job_count() >= MAX_CONCURRENT_JOBS:
        raise JobBusyError(
            f"at most {MAX_CONCURRENT_JOBS} sync job(s) may run; stop the current job first"
        )
    prune_terminal_jobs()
    job_id = str(body.get("id") or uuid.uuid4())
    work = job_dir(job_id)
    work.mkdir(parents=True, exist_ok=True)
    (work / "data").mkdir(exist_ok=True)

    config_path = write_shake_toml(work, body)
    argv = [SHAKE_BIN, config_path.name]
    log_path = work / "sync.log"
    meta = {
        "id": job_id,
        "name": body.get("name") or job_id,
        "status": "starting",
        "created_at": utc_now(),
        "started_at": utc_now(),
        "argv": argv,
        "config_path": str(config_path),
        "log_path": str(log_path),
    }
    write_json(work / "job.json", meta)

    with log_path.open("ab") as log_fh:
        log_fh.write(f"\n--- redis-shake job {job_id} at {utc_now()} ---\n".encode())
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
            cleanup_staging(job_id)
        return meta

    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    meta = job_status(job_id)
    meta["status"] = "stopped"
    meta["finished_at"] = utc_now()
    write_json(job_dir(job_id) / "job.json", meta)
    cleanup_staging(job_id)
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
    "service": "cdc-redisshake",
    "mode": "sync_reader",
    "options": {
        "booleans": [
            "source_cluster",
            "target_cluster",
            "source_tls",
            "target_tls",
            "sync_rdb",
            "sync_aof",
            "prefer_replica",
            "try_diskless",
            "off_reply",
        ],
        "strings": ["rdb_restore_command_behavior"],
    },
    "connection_fields": ["host", "port", "username", "password"],
    "notes": "Uses RedisShake sync_reader + redis_writer; jobs run until stopped or source disconnects.",
    "limits": {
        "max_concurrent_jobs": MAX_CONCURRENT_JOBS,
        "retain_terminal_jobs": RETAIN_TERMINAL_JOBS,
        "cleanup_staging_on_finish": CLEANUP_STAGING,
    },
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
            out = {
                "contract_version": "1",
                "tool_id": "redisshake",
                "service": CAPABILITY["service"],
                "sidecar_ready": True,
                "orchestrator_contract": "/cdc/redisshake/contract",
                "redisshake_binary": SHAKE_BIN if Path(SHAKE_BIN).is_file() else None,
                "limits": CAPABILITY.get("limits"),
            }
            self._json(200, out)
            return
        if path == "/v1/capability":
            out = dict(CAPABILITY)
            out["redisshake_image"] = "ghcr.io/tair-opensource/redisshake"
            if Path(SHAKE_BIN).is_file():
                out["redisshake_binary"] = SHAKE_BIN
            self._json(200, out)
            return
        if path == "/v1/jobs":
            self._json(200, {"jobs": list_jobs()})
            return
        if path.startswith("/v1/jobs/") and path.endswith("/logs"):
            job_id = path.split("/")[3]
            log_path = job_dir(job_id) / "sync.log"
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
            except JobBusyError as exc:
                self._json(409, {"error": str(exc)})
                return
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
    print(f"cdc-redisshake listening on {HOST}:{PORT}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
