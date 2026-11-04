"""Shared job store for the Copy & Sync sidecars (pgcopydb, pgloader, mongocopy, redisshake).

One job per run: each run gets its own id and directory and records the transfer it belongs to
(`profile_id`), so the UI can show run history. A transfer may have one active run at a time;
the newest RETAIN runs per transfer are kept. A watcher thread finalises a run the moment its
process exits, so per-run secrets (passfiles, configs with passwords) are deleted right away.

Copied into each image next to serve.py (compose `additional_contexts: common`). Unit tests:
docker/sidecar_common/test_jobstore.py.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import signal
import subprocess
import threading
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

ACTIVE = ("starting", "running")
JOB_ID = re.compile(r"^[A-Za-z0-9-]{1,64}$")


class BusyError(Exception):
    """A run of the same transfer (or the sidecar's run limit) is already active → HTTP 409."""


class NotFound(Exception):
    pass


def utc_now() -> str:
    # Milliseconds: runs started within the same second must still sort newest-first.
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def write_json(path: Path, data: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(data, indent=2), encoding="utf-8")
    tmp.replace(path)


def read_json(path: Path) -> dict[str, Any]:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def write_secret(path: Path, text: str) -> Path:
    """Create a 0600 file (never world-readable, not even briefly)."""
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        path.unlink()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as fh:
        fh.write(text)
    return path


class JobStore:
    def __init__(self, jobs_dir: Path, log_name: str, retain_per_profile: int = 10, max_active: int = 0):
        self.jobs_dir = jobs_dir
        self.log_name = log_name
        self.retain = max(1, retain_per_profile)
        self.max_active = max_active  # 0 = no global limit
        self._lock = threading.Lock()
        self._procs: dict[str, subprocess.Popen[Any]] = {}
        self._stopping: set[str] = set()

    # ── paths and metadata ────────────────────────────────────────────────
    def job_dir(self, job_id: str) -> Path:
        if not JOB_ID.match(job_id or ""):
            raise NotFound("invalid job id")
        return self.jobs_dir / job_id

    def meta(self, job_id: str) -> dict[str, Any]:
        meta = read_json(self.job_dir(job_id) / "job.json")
        if not meta:
            raise NotFound("job not found")
        meta["id"] = job_id
        # Jobs from before per-run ids used the profile id as the job id.
        meta.setdefault("profile_id", job_id)
        return meta

    def update(self, job_id: str, **fields: Any) -> dict[str, Any]:
        path = self.job_dir(job_id) / "job.json"
        with self._lock:
            meta = read_json(path)
            meta.update(fields)
            write_json(path, meta)
        return meta

    def log_path(self, job_id: str) -> Path:
        return self.job_dir(job_id) / self.log_name

    def tail_log(self, job_id: str, max_bytes: int = 256_000) -> str:
        path = self.log_path(job_id)
        if not path.is_file():
            self.meta(job_id)  # 404 for unknown jobs, empty log for known ones
            return ""
        data = path.read_bytes()[-max_bytes:]
        return data.decode("utf-8", errors="replace")

    def list(self, profile_id: str | None = None) -> list[dict[str, Any]]:
        if not self.jobs_dir.is_dir():
            return []
        out = []
        for child in self.jobs_dir.iterdir():
            if not child.is_dir() or not JOB_ID.match(child.name):
                continue
            try:
                meta = self.meta(child.name)
            except NotFound:
                continue
            if profile_id is None or meta.get("profile_id") == profile_id:
                out.append(meta)
        out.sort(key=lambda m: m.get("created_at") or "", reverse=True)
        return out

    def active(self, profile_id: str | None = None) -> list[dict[str, Any]]:
        return [m for m in self.list(profile_id) if m.get("status") in ACTIVE]

    # ── lifecycle ─────────────────────────────────────────────────────────
    def new_job(self, body: dict[str, Any]) -> tuple[str, Path, dict[str, Any]]:
        """Reserve a run. Older orchestrators sent the profile id as `id`; treat it as profile_id."""
        profile_id = str(body.get("profile_id") or body.get("id") or "").strip() or None
        with self._lock:
            if profile_id and any(m.get("status") in ACTIVE for m in self.list(profile_id)):
                raise BusyError("this transfer is already running; stop it or wait for it to finish")
            if self.max_active and len(self.active()) >= self.max_active:
                raise BusyError(f"at most {self.max_active} run(s) may be active; stop one first")
            job_id = str(uuid.uuid4())
            work = self.jobs_dir / job_id
            work.mkdir(parents=True)
            meta = {
                "id": job_id,
                "profile_id": profile_id or job_id,
                "name": body.get("name") or job_id,
                "status": "starting",
                "created_at": utc_now(),
                "started_at": utc_now(),
                "log_path": str(work / self.log_name),
            }
            write_json(work / "job.json", meta)
        self.prune(profile_id or job_id)
        return job_id, work, meta

    def spawn(
        self,
        job_id: str,
        argv: list[str],
        *,
        env: dict[str, str] | None = None,
        header: str = "",
        on_exit: Callable[[str, int], dict[str, Any] | None] | None = None,
    ) -> dict[str, Any]:
        """Start the tool; a watcher thread records the result and calls on_exit(job_id, code)."""
        work = self.job_dir(job_id)
        with self.log_path(job_id).open("ab") as log_fh:
            if header:
                log_fh.write(header.encode())
            proc = subprocess.Popen(argv, stdout=log_fh, stderr=subprocess.STDOUT, cwd=str(work), env=env, start_new_session=True)
        with self._lock:
            self._procs[job_id] = proc
        meta = self.update(job_id, status="running", pid=proc.pid)
        threading.Thread(target=self._watch, args=(job_id, proc, on_exit), daemon=True).start()
        meta["id"] = job_id
        return meta

    def fail_start(self, job_id: str, error: str) -> dict[str, Any]:
        return self.update(job_id, status="failed", error=error, finished_at=utc_now())

    def _watch(self, job_id: str, proc: subprocess.Popen[Any], on_exit: Callable[[str, int], dict[str, Any] | None] | None) -> None:
        code = proc.wait()
        extra: dict[str, Any] = {}
        if on_exit:
            try:
                extra = on_exit(job_id, code) or {}
            except Exception as exc:  # noqa: BLE001 — cleanup must not lose the result
                extra = {"cleanup_error": f"{type(exc).__name__}: {exc}"}
        with self._lock:
            self._procs.pop(job_id, None)
            stopped = job_id in self._stopping
            self._stopping.discard(job_id)
        status = "stopped" if stopped else ("succeeded" if code == 0 else "failed")
        self.update(job_id, status=status, exit_code=code, finished_at=utc_now(), **extra)

    def stop(self, job_id: str, grace_seconds: float = 10) -> dict[str, Any]:
        meta = self.meta(job_id)
        with self._lock:
            proc = self._procs.get(job_id)
            if proc is not None:
                self._stopping.add(job_id)
        if proc is None:
            if meta.get("status") in ACTIVE:  # process lost (sidecar restarted)
                return self.update(job_id, status="stopped", finished_at=utc_now()) | {"id": job_id}
            return meta
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        deadline = time.time() + grace_seconds
        while proc.poll() is None and time.time() < deadline:
            time.sleep(0.1)
        if proc.poll() is None:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        for _ in range(50):  # let the watcher record the result
            if self.meta(job_id).get("status") not in ACTIVE:
                break
            time.sleep(0.05)
        return self.meta(job_id)

    def prune(self, profile_id: str) -> None:
        finished = [m for m in self.list(profile_id) if m.get("status") not in ACTIVE]
        for meta in finished[self.retain:]:
            shutil.rmtree(self.jobs_dir / meta["id"], ignore_errors=True)

    def recover_on_start(self, cleanup: Callable[[Path], None] | None = None) -> None:
        """After a sidecar restart no process survives: mark active runs failed, drop secrets."""
        for meta in self.list():
            work = self.jobs_dir / meta["id"]
            if cleanup:
                cleanup(work)
            if meta.get("status") in ACTIVE:
                self.update(meta["id"], status="failed", error="the sidecar restarted during the run", finished_at=utc_now())


def probe(argv: list[str], *, env: dict[str, str] | None = None, timeout: float = 20, cwd: str | None = None) -> dict[str, Any]:
    """Run a connection check command; return ok/stdout/error/latency without raising."""
    started = time.monotonic()
    try:
        proc = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=timeout, cwd=cwd)
    except subprocess.TimeoutExpired:
        return {"ok": False, "error": f"no answer within {int(timeout)}s", "latency_ms": int(timeout * 1000)}
    except OSError as exc:
        return {"ok": False, "error": f"cannot run {argv[0]}: {exc}"}
    latency_ms = int((time.monotonic() - started) * 1000)
    if proc.returncode != 0:
        err = "\n".join(line.strip() for line in (proc.stderr or proc.stdout).strip().splitlines() if line.strip())
        return {"ok": False, "error": err[-800:] or f"exit code {proc.returncode}", "latency_ms": latency_ms}
    return {"ok": True, "stdout": proc.stdout, "latency_ms": latency_ms}


def plural(n: Any, word: str) -> str:
    """1 table · 3 tables"""
    try:
        count = int(n)
    except (TypeError, ValueError):
        return f"? {word}s"
    return f"{count:,} {word}{'' if count == 1 else 's'}"


def fmt_bytes(n: Any) -> str:
    try:
        size = float(n)
    except (TypeError, ValueError):
        return ""
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if size < 1024 or unit == "TB":
            return f"{size:.0f} {unit}" if unit == "B" or size >= 10 else f"{size:.1f} {unit}"
        size /= 1024
    return ""
