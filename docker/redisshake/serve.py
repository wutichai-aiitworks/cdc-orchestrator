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


def db_numbers(value: Any) -> list[int]:
    """Source databases to sync, sorted and unique. Empty means every database."""
    if value in (None, ""):
        return []
    if not isinstance(value, list):
        raise BadRequest("source_dbs must be a list of database numbers")
    out: set[int] = set()
    for v in value:
        if isinstance(v, bool) or not isinstance(v, int) or not 0 <= v <= 9999:
            raise BadRequest(f"source_dbs: {v!r} is not a database number")
        out.add(v)
    return sorted(out)


def filter_lines(options: dict[str, Any], src_cluster: bool, tgt_cluster: bool) -> list[str]:
    """[filter] for the database choice: allow_db, plus a function that moves one source database
    into another target database (shake.call's first argument is the target database)."""
    dbs = db_numbers(options.get("source_dbs"))
    target_db = options.get("target_db")
    if src_cluster and any(dbs):
        raise BadRequest("A Redis Cluster source has only database 0")
    if target_db is not None:
        if isinstance(target_db, bool) or not isinstance(target_db, int) or not 0 <= target_db <= 9999:
            raise BadRequest("target_db must be a database number")
        if len(dbs) != 1:
            raise BadRequest("target_db needs exactly one source database")
        if tgt_cluster and target_db != 0:
            raise BadRequest("A Redis Cluster target has only database 0")
        if dbs[0] == target_db:
            target_db = None
    if not dbs:
        return []
    lines = ["[filter]", f"allow_db = [{', '.join(map(str, dbs))}]"]
    if target_db is not None:
        lines.append(f'function = """\nshake.call({int(target_db)}, ARGV)\n"""')
    return lines + [""]


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

    lines.extend(filter_lines(options, src_cluster, tgt_cluster))

    advanced = []
    behavior = (options.get("rdb_restore_command_behavior") or "panic").strip()
    if behavior in ("panic", "rewrite", "skip"):
        advanced.append(f"rdb_restore_command_behavior = {toml_str(behavior)}")
    # Cluster targets have only db 0, so RedisShake's own FLUSHALL has the right scope there.
    # Single servers are emptied by empty_target() instead: RedisShake would flush every database.
    if wants_empty(options) and tgt_cluster:
        advanced.append("empty_db_before_sync = true")
    if advanced:
        lines.extend(["[advanced]", *advanced, ""])

    # Holds both passwords: 0600, deleted when the sync ends.
    return write_secret(work / "shake.toml", "\n".join(lines))


def wants_empty(options: dict[str, Any]) -> bool:
    """Empty the target first: only before a full copy (nothing would refill it otherwise)."""
    return bool(options.get("empty_target")) and options.get("sync_rdb") is not False


def empty_scope(options: dict[str, Any]) -> list[int] | None:
    """Target databases a full copy writes; None = every database (the sync copies them all)."""
    dbs = db_numbers(options.get("source_dbs"))
    if not dbs:
        return None
    t = options.get("target_db")
    if isinstance(t, int) and not isinstance(t, bool) and len(dbs) == 1:
        return [t]
    return dbs


def empty_target(conn: dict[str, Any], tls: bool, dbs: list[int] | None, timeout: float = 30) -> dict[str, Any]:
    """FLUSHDB the given target databases (FLUSHALL when None). Returns what was removed."""
    host, port = (conn.get("host") or "").strip(), int(str(conn.get("port") or "6379"))
    try:
        sock = socket.create_connection((host, port), timeout=timeout)
        if tls:
            ctx = ssl.create_default_context()
            ctx.check_hostname = False
            ctx.verify_mode = ssl.CERT_NONE  # same as RedisShake's tls = true
            sock = ctx.wrap_socket(sock, server_hostname=host)
        with sock, sock.makefile("rwb") as f:
            def cmd(*args: str) -> Any:
                f.write(_encode(*args))
                f.flush()
                return _read(f)

            user, password = conn.get("username") or "", conn.get("password") or ""
            if password:
                cmd(*(["AUTH", user, password] if user else ["AUTH", password]))
            before = keyspace_dbs(parse_info(cmd("INFO", "keyspace")))
            if dbs is None:
                cmd("FLUSHALL")
                removed = sum(before.values())
            else:
                for db in dbs:
                    cmd("SELECT", str(db))
                    cmd("FLUSHDB")
                removed = sum(before.get(db, 0) for db in dbs)
    except RespError as exc:
        raise BadRequest(
            f"could not empty the target: {exc}. The target user needs FLUSHDB/FLUSHALL "
            "(and SELECT) permission, and those commands must not be renamed or disabled."
        ) from exc
    except (OSError, ConnectionError) as exc:
        raise BadRequest(f"could not empty the target: {exc or type(exc).__name__}") from exc
    return {"scope": "all" if dbs is None else dbs, "keys_removed": removed}


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
        options = body.get("options") or {}
        # How this run was started (no secrets): "once" = full copy then exit, "cdc" = follow changes.
        live = options.get("sync_aof") is not False
        STORE.update(job_id, argv=argv, config_path=str(config),
                     mode="cdc" if live else "once", snapshot=options.get("sync_rdb") is not False)
        header = f"\n--- redis-shake job {job_id} at {utc_now()} ---\n"
        if wants_empty(options):
            tgt_cluster_on, _ = side_options(options, "target")
            if tgt_cluster_on:
                emptied: dict[str, Any] = {"scope": "all", "keys_removed": None, "by": "redis-shake"}
                header += "Target is a cluster: RedisShake empties it (FLUSHALL) before the full copy.\n"
            else:
                scope = empty_scope(options)
                emptied = empty_target(body.get("target") or {}, bool(options.get("target_tls")), scope)
                where = "every database" if scope is None else ", ".join(f"db {d}" for d in scope)
                header += f"Emptied the target before the full copy: {where}, {emptied['keys_removed']} key(s) removed.\n"
            STORE.update(job_id, emptied=emptied)
        header += f"{' '.join(argv)}\n"
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


def keyspace_dbs(info: dict[str, str]) -> dict[int, int]:
    """Keys per database number from INFO keyspace (databases with no keys are not listed)."""
    out: dict[int, int] = {}
    for k, v in info.items():
        if k.startswith("db") and k[2:].isdigit() and "keys=" in v:
            out[int(k[2:])] = int(v.split("keys=", 1)[1].split(",", 1)[0])
    return dict(sorted(out.items()))


def keyspace_keys(info: dict[str, str]) -> int:
    return sum(keyspace_dbs(info).values())


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
            dbs = keyspace_dbs(parse_info(cmd("INFO", "keyspace")))
            keys = sum(dbs.values())
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
        "dbs": {str(db): n for db, n in dbs.items()},
        "facts": [f"Redis {server.get('redis_version', '?')}", mode, plural(keys, "key"), fmt_bytes(memory.get("used_memory"))],
    }
    if dbs:
        out["facts"].append("Keys in " + ", ".join(f"db{db} ({n:,})" for db, n in dbs.items()))
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
