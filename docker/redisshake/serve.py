#!/usr/bin/env python3
"""HTTP sidecar for Redis sync (RedisShake). Contract: sidecar_common/http_api.py.

One job per run (history per transfer); a sync runs until it is stopped. The shake.toml holds
both passwords: 0600 and deleted, with the staging data, when the sync ends.
"""

from __future__ import annotations

import os
import shutil
import socket
import ssl
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "sidecar_common"))  # local tests

from http_api import Tool, serve  # noqa: E402
from jobstore import JobStore, fmt_bytes, plural, utc_now, write_secret  # noqa: E402
from probes import BadRequest, describe, hint_for, norm_conn  # noqa: E402

WORK_ROOT = Path(os.environ.get("REDIS_SHAKE_WORK_ROOT", "/var/lib/redisshake"))
JOBS_DIR = WORK_ROOT / "jobs"
API_TOKEN = os.environ.get("REDIS_SHAKE_API_TOKEN", "").strip()
HOST = os.environ.get("REDIS_SHAKE_API_HOST", "0.0.0.0")
PORT = int(os.environ.get("REDIS_SHAKE_API_PORT", "8086"))
SHAKE_BIN = os.environ.get("REDIS_SHAKE_BIN", "/app/redis-shake")
MAX_CONCURRENT_JOBS = max(1, int(os.environ.get("REDIS_SHAKE_MAX_CONCURRENT_JOBS", "1")))
RETAIN = max(1, int(os.environ.get("REDIS_SHAKE_RETAIN_JOBS", "15")))
CLEANUP_STAGING = os.environ.get("REDIS_SHAKE_CLEANUP_STAGING", "true").strip().lower() in ("1", "true", "yes", "on")

STORE = JobStore(JOBS_DIR, "sync.log", retain_per_profile=RETAIN, max_active=MAX_CONCURRENT_JOBS)


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

    # Holds both passwords: 0600, deleted when the sync ends.
    return write_secret(work / "shake.toml", "\n".join(lines))


def cleanup(work: Path) -> None:
    (work / "shake.toml").unlink(missing_ok=True)
    if CLEANUP_STAGING:
        shutil.rmtree(work / "data", ignore_errors=True)


def start_job(body: dict[str, Any]) -> dict[str, Any]:
    job_id, work, meta = STORE.new_job(body)
    try:
        (work / "data").mkdir(exist_ok=True)
        config = write_shake_toml(work, body)
        argv = [SHAKE_BIN, config.name]
        STORE.update(job_id, argv=argv, config_path=str(config))
        header = f"\n--- redis-shake job {job_id} at {utc_now()} ---\n{' '.join(argv)}\n"
        return STORE.spawn(job_id, argv, header=header, on_exit=lambda jid, code: cleanup(work))
    except Exception as exc:
        cleanup(work)
        STORE.fail_start(job_id, f"{type(exc).__name__}: {exc}")
        raise


# ── connection test: a few RESP commands over a plain (or TLS) socket ────────


def _encode(*args: str) -> bytes:
    out = [f"*{len(args)}\r\n".encode()]
    for a in args:
        b = a.encode()
        out.append(f"${len(b)}\r\n".encode() + b + b"\r\n")
    return b"".join(out)


class RespError(Exception):
    pass


def _read(f: Any) -> Any:
    line = f.readline()
    if not line:
        raise ConnectionError("connection closed by the server")
    kind, rest = line[:1], line[1:].rstrip(b"\r\n")
    if kind == b"+":
        return rest.decode()
    if kind == b"-":
        raise RespError(rest.decode())
    if kind == b":":
        return int(rest)
    if kind == b"$":
        n = int(rest)
        if n < 0:
            return None
        data = f.read(n + 2)
        return data[:-2].decode("utf-8", errors="replace")
    if kind == b"*":
        return [_read(f) for _ in range(max(0, int(rest)))]
    raise RespError(f"unexpected reply {line[:40]!r}")


def parse_info(text: str) -> dict[str, str]:
    out = {}
    for line in (text or "").splitlines():
        if ":" in line and not line.startswith("#"):
            k, v = line.split(":", 1)
            out[k.strip()] = v.strip()
    return out


def keyspace_keys(info: dict[str, str]) -> int:
    total = 0
    for k, v in info.items():
        if k.startswith("db") and "keys=" in v:
            total += int(v.split("keys=", 1)[1].split(",", 1)[0])
    return total


def redis_test(conn: dict[str, str], tls: bool, cluster: bool, timeout: float = 8) -> dict[str, Any]:
    target = f"{conn['host']}:{conn['port']}"
    try:
        sock = socket.create_connection((conn["host"], int(conn["port"])), timeout=timeout)
        if tls:
            ctx = ssl.create_default_context()
            ctx.check_hostname = False
            ctx.verify_mode = ssl.CERT_NONE  # same as RedisShake's tls = true
            sock = ctx.wrap_socket(sock, server_hostname=conn["host"])
        with sock, sock.makefile("rwb") as f:
            def cmd(*args: str) -> Any:
                f.write(_encode(*args))
                f.flush()
                return _read(f)

            if conn["password"]:
                cmd(*(["AUTH", conn["username"], conn["password"]] if conn["username"] else ["AUTH", conn["password"]]))
            cmd("PING")
            server = parse_info(cmd("INFO", "server"))
            keys = keyspace_keys(parse_info(cmd("INFO", "keyspace")))
            memory = parse_info(cmd("INFO", "memory"))
    except RespError as exc:
        msg = str(exc)
        return {"ok": False, "target": target, "error": msg, "hint": hint_for(msg) or ("The server needs a password." if "NOAUTH" in msg else None)}
    except ssl.SSLError as exc:
        return {"ok": False, "target": target, "error": f"TLS: {exc}", "hint": "The TLS setting does not match the server. Turn TLS on or off to match it."}
    except (OSError, ConnectionError) as exc:
        msg = str(exc) or type(exc).__name__
        return {"ok": False, "target": target, "error": msg, "hint": hint_for(msg) or hint_for("connection refused")}
    mode = server.get("redis_mode", "standalone")
    out: dict[str, Any] = {
        "ok": True,
        "target": target,
        "server_version": server.get("redis_version"),
        "keys": keys,
        "facts": [f"Redis {server.get('redis_version', '?')}", mode, plural(keys, "key"), fmt_bytes(memory.get("used_memory"))],
    }
    if cluster and mode != "cluster":
        out["warning"] = "“Cluster” is on, but this server is not a cluster node. Turn it off."
    elif not cluster and mode == "cluster":
        out["warning"] = "This server is a cluster node. Turn on “Cluster” so every shard is synced."
    return out


def test_connection(body: dict[str, Any]) -> dict[str, Any]:
    conn = norm_conn(body.get("connection"), default_port="6379", need_database=False, need_user=False)
    side = body.get("side") or "source"
    if side not in ("source", "target"):
        raise BadRequest("side must be source or target")
    options = body.get("options") or {}
    out = redis_test(conn, bool(options.get(f"{side}_tls")), bool(options.get(f"{side}_cluster")))
    if out.get("ok") and side == "target" and out.get("keys"):
        out.setdefault("warning", "The target already has keys. Choose below what happens when a key exists on both.")
    out["target"] = describe(conn) if conn["username"] else out["target"]
    return out


def capability() -> dict[str, Any]:
    return {
        "connection_fields": ["host", "port", "username", "password"],
        "max_concurrent_jobs": MAX_CONCURRENT_JOBS,
        "retain_runs_per_profile": RETAIN,
        "redisshake_bin": SHAKE_BIN,
    }


TOOL = Tool(tool_id="redisshake", service="cdc-redisshake", store=STORE, capability=capability, start=start_job,
            test=test_connection, api_token=API_TOKEN)

if __name__ == "__main__":
    STORE.recover_on_start(cleanup)
    serve(TOOL, HOST, PORT)
