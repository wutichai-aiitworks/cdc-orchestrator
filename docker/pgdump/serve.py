#!/usr/bin/env python3
"""HTTP sidecar for PostgreSQL remote dump (pg_dump) and dump → restore (pg_restore).

Same job contract as the other migration sidecars (/health, /v1/discover, /v1/capability,
/v1/jobs …) plus GET /v1/jobs/{id}/file to download the artifact, POST /v1/jobs/{id}/send to upload it to
one ad-hoc SFTP server, and DELETE /v1/jobs/{id} to remove it.

Secrets: passwords never appear in argv, job.json or logs. Each job gets a 0600 .pgpass with
exact host:port:*:user lines for source and target; it is deleted when the job ends (and on
sidecar start, for jobs a crash left behind).
"""

from __future__ import annotations

import base64
import hashlib
import io
import json
import os
import re
import shutil
import signal
import subprocess
import tempfile
import threading
import time
import uuid
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse
import posixpath

WORK_ROOT = Path(os.environ.get("PGDUMP_WORK_ROOT", "/var/lib/pgdump"))
JOBS_DIR = WORK_ROOT / "jobs"
API_TOKEN = os.environ.get("PGDUMP_API_TOKEN", "").strip()
HOST = os.environ.get("PGDUMP_API_HOST", "0.0.0.0")
PORT = int(os.environ.get("PGDUMP_API_PORT", "8089"))
MAX_CONCURRENT = int(os.environ.get("PGDUMP_MAX_CONCURRENT", "2"))
MIN_FREE_MB = int(os.environ.get("PGDUMP_MIN_FREE_MB", "1024"))
TIMEOUT_SECONDS = int(os.environ.get("PGDUMP_TIMEOUT_SECONDS", "0"))  # 0 = no limit
RETENTION_HOURS = int(os.environ.get("PGDUMP_RETENTION_HOURS", "72"))
PG_DUMP = os.environ.get("PGDUMP_PG_DUMP", "pg_dump")
PG_RESTORE = os.environ.get("PGDUMP_PG_RESTORE", "pg_restore")
PSQL = os.environ.get("PGDUMP_PSQL", "psql")

MODES = ("dump", "dump_restore")
ACTIVE = ("starting", "running")
FORMATS = {"custom": ("c", "dump"), "plain": ("p", "sql"), "tar": ("t", "tar")}
SSL_MODES = ("disable", "allow", "prefer", "require", "verify-ca", "verify-full")

BOOL_DUMP: dict[str, str] = {
    "schema_only": "--schema-only",
    "data_only": "--data-only",
    "no_owner": "--no-owner",
    "no_privileges": "--no-privileges",
    "no_comments": "--no-comments",
    "no_publications": "--no-publications",
    "no_subscriptions": "--no-subscriptions",
    "clean": "--clean",
    "if_exists": "--if-exists",
    "create": "--create",
    "serializable_deferrable": "--serializable-deferrable",
}
LIST_DUMP: dict[str, str] = {
    "schemas": "--schema",
    "exclude_schemas": "--exclude-schema",
    "exclude_tables": "--exclude-table",
    "exclude_table_data": "--exclude-table-data",
}
BOOL_RESTORE: dict[str, str] = {
    "restore_clean": "--clean",
    "restore_if_exists": "--if-exists",
    "restore_no_owner": "--no-owner",
    "restore_no_privileges": "--no-privileges",
    "restore_single_transaction": "--single-transaction",
    "restore_exit_on_error": "--exit-on-error",
}
OTHER_OPTIONS = {"format", "compress", "lock_wait_timeout_ms", "restore_jobs", "source_sslmode",
                 "target_sslmode", "keep_local"}
KNOWN_OPTIONS = set(BOOL_DUMP) | set(LIST_DUMP) | set(BOOL_RESTORE) | OTHER_OPTIONS

# pg_dump object pattern: identifiers, quotes, wildcards, dots, spaces. Never a leading '-'.
OBJECT_PATTERN = re.compile(r'^(?!-)[\w.*?$" -]{1,256}$')
JOB_ID = re.compile(r"^[A-Za-z0-9-]{1,64}$")
HOST_RE = re.compile(r"^[A-Za-z0-9._:\-\[\]]{1,253}$")

_lock = threading.Lock()
_procs: dict[str, subprocess.Popen[Any]] = {}
_runtime: dict[str, dict[str, Any]] = {}
_stop_requested: set[str] = set()
# Ad-hoc sends of a finished job's file (POST /v1/jobs/{id}/send): one at a time per job.
_sending: dict[str, dict[str, Any]] = {}
_send_stop: set[str] = set()


class BadRequest(Exception):
    def __init__(self, message: str, code: int = 400) -> None:
        super().__init__(message)
        self.code = code


# ── pure helpers (unit-tested) ──────────────────────────────────────────────


def utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def is_postgres(db_type: Any) -> bool:
    return str(db_type or "postgres").strip().lower() in ("postgres", "postgresql", "pg")


def norm_conn(conn: Any, role: str) -> dict[str, str]:
    """Validate a connection and return host/port/database/username/password as strings."""
    if not isinstance(conn, dict) or not conn:
        raise BadRequest(f"{role} connection is required")
    if not is_postgres(conn.get("type")):
        raise BadRequest(f"{role} must be PostgreSQL")
    host = str(conn.get("host") or "").strip()
    if not host or not HOST_RE.match(host):
        raise BadRequest(f"{role}.host is required and must be a hostname or IP")
    port = str(conn.get("port") or "5432").strip()
    if not port.isdigit() or not 1 <= int(port) <= 65535:
        raise BadRequest(f"{role}.port must be 1-65535")
    database = str(conn.get("database") or "").strip()
    if not database:
        raise BadRequest(f"{role}.database is required")
    user = str(conn.get("username") or conn.get("user") or "").strip()
    if not user:
        raise BadRequest(f"{role}.username is required")
    return {"host": host, "port": port, "database": database, "username": user,
            "password": str(conn.get("password") or "")}


def describe(conn: dict[str, str]) -> str:
    """Credential-free target description for job status."""
    return f"{conn['username']}@{conn['host']}:{conn['port']}/{conn['database']}"


def _pattern_list(options: dict[str, Any], key: str) -> list[str]:
    raw = options.get(key)
    if raw in (None, "", []):
        return []
    if isinstance(raw, str):
        raw = raw.split(",")
    if not isinstance(raw, list):
        raise BadRequest(f"options.{key} must be a list of strings")
    if len(raw) > 200:
        raise BadRequest(f"options.{key} accepts at most 200 entries")
    out = []
    for item in raw:
        s = str(item).strip()
        if not s:
            continue
        if not OBJECT_PATTERN.match(s):
            raise BadRequest(f"options.{key} entry {s[:40]!r} is not a valid object pattern")
        out.append(s)
    return out


def _int_opt(options: dict[str, Any], key: str, lo: int, hi: int) -> int | None:
    raw = options.get(key)
    if raw is None or raw == "":
        return None
    if isinstance(raw, bool):
        raise BadRequest(f"options.{key} must be an integer")
    try:
        val = int(raw)
    except (TypeError, ValueError):
        raise BadRequest(f"options.{key} must be an integer") from None
    if not lo <= val <= hi:
        raise BadRequest(f"options.{key} must be between {lo} and {hi}")
    return val


def validate_request(body: dict[str, Any]) -> dict[str, Any]:
    """Normalise a POST /v1/jobs body. Unknown option keys are rejected: a typo must never
    silently turn a narrow dump into a full one."""
    mode = str(body.get("mode") or "dump").strip()
    if mode not in MODES:
        raise BadRequest(f"mode must be one of {list(MODES)}")
    options = body.get("options") or {}
    if not isinstance(options, dict):
        raise BadRequest("options must be an object")
    unknown = sorted(k for k, v in options.items() if k not in KNOWN_OPTIONS and v not in (None, False, "", []))
    if unknown:
        raise BadRequest(f"unknown options: {unknown}")

    source = norm_conn(body.get("source"), "source")
    target = norm_conn(body.get("target"), "target") if mode == "dump_restore" else None

    fmt = str(options.get("format") or "custom").strip().lower()
    if fmt not in FORMATS:
        raise BadRequest(f"options.format must be one of {list(FORMATS)}")
    if mode == "dump_restore" and fmt != "custom":
        raise BadRequest("dump_restore requires format=custom (pg_restore reads the archive)")
    flags = {k: bool(options.get(k)) for k in list(BOOL_DUMP) + list(BOOL_RESTORE)}
    if flags["schema_only"] and flags["data_only"]:
        raise BadRequest("schema_only and data_only are mutually exclusive")
    if flags["if_exists"] and not flags["clean"]:
        raise BadRequest("if_exists requires clean")
    if flags["restore_if_exists"] and not flags["restore_clean"]:
        raise BadRequest("restore_if_exists requires restore_clean")
    compress = _int_opt(options, "compress", 0, 9)
    if fmt == "tar" and compress:
        raise BadRequest("tar format does not support compression")
    restore_jobs = _int_opt(options, "restore_jobs", 1, 32)
    if restore_jobs and restore_jobs > 1 and flags["restore_single_transaction"]:
        raise BadRequest("restore_jobs > 1 cannot be combined with restore_single_transaction")
    for key in ("source_sslmode", "target_sslmode"):
        val = options.get(key)
        if val not in (None, "") and str(val) not in SSL_MODES:
            raise BadRequest(f"options.{key} must be one of {list(SSL_MODES)}")

    include_tables = _pattern_list({"t": body.get("include_tables")}, "t")
    return {
        "mode": mode,
        "name": str(body.get("name") or "").strip()[:120],
        "profile_id": str(body.get("profile_id") or "").strip()[:64] or None,
        "source": source,
        "target": target,
        "include_tables": include_tables,
        "format": fmt,
        "compress": compress,
        "lock_wait_timeout_ms": _int_opt(options, "lock_wait_timeout_ms", 1, 86_400_000),
        "restore_jobs": restore_jobs,
        "flags": flags,
        "lists": {k: _pattern_list(options, k) for k in LIST_DUMP},
        "source_sslmode": options.get("source_sslmode") or None,
        "target_sslmode": options.get("target_sslmode") or None,
        "destinations": norm_destinations(body.get("destinations")),
        "keep_local": bool(options.get("keep_local")),
    }


MAX_DESTINATIONS = 5
FINGERPRINT = re.compile(r"^SHA256:[A-Za-z0-9+/]{43}=?$")
SFTP_TIMEOUT = int(os.environ.get("PGDUMP_SFTP_TIMEOUT_SECONDS", "30"))


def norm_destinations(raw: Any) -> list[dict[str, Any]]:
    """Validate SFTP destinations. Secrets stay in memory: never written to job.json or logs."""
    if raw in (None, []):
        return []
    if not isinstance(raw, list):
        raise BadRequest("destinations must be a list")
    if len(raw) > MAX_DESTINATIONS:
        raise BadRequest(f"at most {MAX_DESTINATIONS} destinations")
    out = []
    for i, d in enumerate(raw):
        where = f"destinations[{i}]"
        if not isinstance(d, dict):
            raise BadRequest(f"{where} must be an object")
        host = str(d.get("host") or "").strip()
        if not host or not HOST_RE.match(host):
            raise BadRequest(f"{where}.host is required and must be a hostname or IP")
        port = str(d.get("port") or "22").strip()
        if not port.isdigit() or not 1 <= int(port) <= 65535:
            raise BadRequest(f"{where}.port must be 1-65535")
        user = str(d.get("username") or "").strip()
        if not user or any(c in user for c in "\r\n\0"):
            raise BadRequest(f"{where}.username is required")
        remote_dir = str(d.get("remote_dir") or "").strip()
        if not remote_dir or len(remote_dir) > 1024 or any(c in remote_dir for c in "\r\n\0"):
            raise BadRequest(f"{where}.remote_dir is required (directory on the SFTP server)")
        password = str(d.get("password") or "")
        private_key = str(d.get("private_key") or "")
        auth = str(d.get("auth") or ("key" if private_key else "password")).strip()
        if auth not in ("key", "password"):
            raise BadRequest(f"{where}.auth must be 'key' or 'password'")
        if auth == "key" and "PRIVATE KEY" not in private_key:
            raise BadRequest(f"{where}.private_key is required for auth=key (PEM / OpenSSH format)")
        if auth == "password" and not password:
            raise BadRequest(f"{where}.password is required for auth=password")
        fp = str(d.get("host_key_fingerprint") or "").strip()
        if fp and not FINGERPRINT.match(fp):
            raise BadRequest(f"{where}.host_key_fingerprint must look like SHA256:<43 base64 chars> "
                             "(ssh-keygen -lf / ssh-keyscan output)")
        out.append({
            "name": str(d.get("name") or host).strip()[:60],
            "host": host, "port": int(port), "username": user, "auth": auth,
            "password": password if auth == "password" else "",
            "private_key": private_key if auth == "key" else "",
            "passphrase": str(d.get("passphrase") or "") if auth == "key" else "",
            "remote_dir": remote_dir, "host_key_fingerprint": fp.rstrip("="),
        })
    return out


def describe_destination(d: dict[str, Any]) -> dict[str, Any]:
    """Credential-free view for job status."""
    return {"name": d["name"], "host": d["host"], "port": d["port"], "username": d["username"],
            "auth": d["auth"], "remote_dir": d["remote_dir"],
            "host_key_pinned": bool(d["host_key_fingerprint"])}


def key_fingerprint(key_blob: bytes) -> str:
    """OpenSSH-style SHA256 fingerprint (same as ssh-keygen -lf), without base64 padding."""
    return "SHA256:" + base64.b64encode(hashlib.sha256(key_blob).digest()).decode().rstrip("=")


def remote_join(remote_dir: str, name: str) -> str:
    return posixpath.join(remote_dir, name)


class UploadStopped(Exception):
    pass


def load_private_key(text: str, passphrase: str) -> Any:
    import paramiko  # lazy: only the sidecar image needs it

    last: Exception | None = None
    for cls in (paramiko.Ed25519Key, paramiko.ECDSAKey, paramiko.RSAKey):
        try:
            return cls.from_private_key(io.StringIO(text), password=passphrase or None)
        except paramiko.PasswordRequiredException:
            raise BadRequest("private key is encrypted; set passphrase") from None
        except Exception as exc:  # noqa: BLE001 - try the next key type
            last = exc
    raise BadRequest(f"invalid private key or wrong passphrase ({type(last).__name__})")


def sftp_upload(dest: dict[str, Any], local: Path, sha_hex: str, progress: Any, stopped: Any) -> dict[str, Any]:
    """Upload `local` to dest.remote_dir atomically (.part, then rename) plus a .sha256 file."""
    import paramiko

    observed: dict[str, str] = {}

    class PinPolicy(paramiko.MissingHostKeyPolicy):
        def missing_host_key(self, client: Any, hostname: str, key: Any) -> None:
            fp = key_fingerprint(key.asbytes())
            observed["fingerprint"] = fp
            want = dest["host_key_fingerprint"]
            if want and want != fp:
                raise BadRequest(f"host key mismatch for {hostname}: expected {want}, got {fp}")

    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(PinPolicy())
    kwargs: dict[str, Any] = {"hostname": dest["host"], "port": dest["port"], "username": dest["username"],
                              "timeout": SFTP_TIMEOUT, "banner_timeout": SFTP_TIMEOUT,
                              "auth_timeout": SFTP_TIMEOUT, "allow_agent": False, "look_for_keys": False}
    if dest["auth"] == "key":
        kwargs["pkey"] = load_private_key(dest["private_key"], dest["passphrase"])
    else:
        kwargs["password"] = dest["password"]
    result: dict[str, Any] = {}
    try:
        client.connect(**kwargs)
        result["host_key_fingerprint"] = observed.get("fingerprint")
        result["host_key_verified"] = bool(dest["host_key_fingerprint"])
        sftp = client.open_sftp()
        _sftp_makedirs(sftp, dest["remote_dir"])
        final = remote_join(dest["remote_dir"], local.name)
        part = final + ".part"

        def cb(sent: int, total: int) -> None:
            progress(sent, total)
            if stopped():
                raise UploadStopped()

        sftp.put(str(local), part, callback=cb, confirm=True)
        size = sftp.stat(part).st_size
        if size != local.stat().st_size:
            raise RuntimeError(f"size mismatch after upload ({size} != {local.stat().st_size})")
        _sftp_replace(sftp, part, final)
        with sftp.open(final + ".sha256", "w") as fh:
            fh.write(f"{sha_hex}  {local.name}\n")
        result.update(remote_path=final, bytes=size)
        sftp.close()
        return result
    finally:
        client.close()


def _sftp_makedirs(sftp: Any, remote_dir: str) -> None:
    path = ""
    for part in remote_dir.split("/"):
        path = "/" if (part == "" and path == "") else (posixpath.join(path, part) if path else part)
        if part in ("", "."):
            continue
        try:
            sftp.stat(path)
        except OSError:
            sftp.mkdir(path)


def _sftp_replace(sftp: Any, src: str, dst: str) -> None:
    try:
        sftp.posix_rename(src, dst)  # atomic overwrite (OpenSSH extension)
    except (OSError, AttributeError):
        try:
            sftp.remove(dst)
        except OSError:
            pass
        sftp.rename(src, dst)


UPLOADER = sftp_upload  # replaced in unit tests


def pgpass_escape(value: str) -> str:
    return value.replace("\\", "\\\\").replace(":", "\\:")


def pgpass_content(conns: list[dict[str, str]]) -> str:
    """Exact host:port:*:user lines, so the source password is never offered to the target."""
    lines = []
    for c in conns:
        if c and c.get("password"):
            lines.append(":".join([pgpass_escape(c["host"]), pgpass_escape(c["port"]), "*",
                                   pgpass_escape(c["username"]), pgpass_escape(c["password"])]))
    return "\n".join(lines) + ("\n" if lines else "")


def conninfo_quote(value: str) -> str:
    return "'" + value.replace("\\", "\\\\").replace("'", "\\'") + "'"


def libpq_env(base: dict[str, str], conn: dict[str, str], passfile: Path | None,
              sslmode: str | None) -> dict[str, str]:
    """Child env: inherited PG* variables are dropped so nothing leaks between jobs."""
    env = {k: v for k, v in base.items() if not k.startswith("PG")}
    env.update({"PGHOST": conn["host"], "PGPORT": conn["port"], "PGUSER": conn["username"],
                "PGDATABASE": conn["database"], "PGAPPNAME": "cdc-pgdump"})
    if passfile is not None:
        env["PGPASSFILE"] = str(passfile)
    if sslmode:
        env["PGSSLMODE"] = sslmode
    return env


def build_dump_argv(req: dict[str, Any], out_file: Path) -> list[str]:
    fmt_flag = FORMATS[req["format"]][0]
    argv = [PG_DUMP, f"--format={fmt_flag}", f"--file={out_file}", "--no-password", "--verbose"]
    if req["compress"] is not None:
        argv.append(f"--compress={req['compress']}")
    for key, flag in BOOL_DUMP.items():
        if req["flags"][key]:
            argv.append(flag)
    for key, flag in LIST_DUMP.items():
        argv.extend(f"{flag}={v}" for v in req["lists"][key])
    argv.extend(f"--table={t}" for t in req["include_tables"])
    if req["lists"]["schemas"] or req["include_tables"]:
        # Without this a mistyped include pattern yields an empty dump that "succeeds".
        argv.append("--strict-names")
    if req["lock_wait_timeout_ms"]:
        argv.append(f"--lock-wait-timeout={req['lock_wait_timeout_ms']}")
    return argv


def build_restore_argv(req: dict[str, Any], archive: Path) -> list[str]:
    # --dbname as a quoted conninfo: a database name can never be parsed as extra keywords.
    argv = [PG_RESTORE, f"--dbname=dbname={conninfo_quote(req['target']['database'])}",
            "--no-password", "--verbose"]
    for key, flag in BOOL_RESTORE.items():
        if req["flags"][key]:
            argv.append(flag)
    if req["restore_jobs"]:
        argv.append(f"--jobs={req['restore_jobs']}")
    argv.append(str(archive))
    return argv


def artifact_name(req: dict[str, Any], job_id: str) -> str:
    base = re.sub(r"[^A-Za-z0-9_-]+", "-", req["name"] or req["source"]["database"]).strip("-")[:60] or "dump"
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    ext = FORMATS[req["format"]][1]
    if req["format"] == "plain" and req["compress"]:
        ext = "sql.gz"
    return f"{base}-{stamp}-{job_id[:8]}.{ext}"


IGNORED_ERRORS = re.compile(r"errors ignored on restore: (\d+)")
PLAIN_IDENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_$]{0,62}$")
TOC_SCHEMA_TYPES = {"TABLE", "SEQUENCE", "VIEW", "FUNCTION", "TYPE", "DOMAIN", "INDEX"}


def toc_schemas(toc: str) -> tuple[list[str], list[str]]:
    """Schemas owning objects in a `pg_restore --list` TOC.

    `pg_dump --table` dumps the tables but never their CREATE SCHEMA, so a filtered dump cannot be
    restored into a fresh database. Lines look like `216; 1259 16385 TABLE shop orders postgres`.
    Returns (plain identifiers, names skipped because they are not plain identifiers).
    """
    found: list[str] = []
    skipped: list[str] = []
    for line in toc.splitlines():
        if not line or line.startswith(";"):
            continue
        tokens = line.split()
        if len(tokens) < 6 or not tokens[0].endswith(";") or tokens[3] not in TOC_SCHEMA_TYPES:
            continue
        if tokens[4] in ("DATA", "SET", "OWNED", "ATTACH"):
            continue  # TABLE DATA / SEQUENCE SET / SEQUENCE OWNED BY / … ATTACH: covered by the base entry
        schema = tokens[4]
        if schema == "-":
            continue
        bucket = found if PLAIN_IDENT.match(schema) else skipped
        if schema not in bucket:
            bucket.append(schema)
    return found, skipped


def create_schemas_sql(schemas: list[str]) -> str:
    return "".join(f'CREATE SCHEMA IF NOT EXISTS "{s}";' for s in schemas)


def first_error(log_tail: str) -> str | None:
    for line in log_tail.splitlines():
        if "error:" in line:
            return line.strip()[:500]
    return None


def hint_for(log_tail: str) -> str | None:
    s = log_tail.lower()
    if "server version mismatch" in s:
        return "pg_dump is older than the server; rebuild cdc-pgdump with a newer PG_MAJOR."
    if "password authentication failed" in s or "no password supplied" in s:
        return "Authentication failed; check the profile username/password."
    if any(x in s for x in ("could not translate host name", "connection refused", "timeout expired",
                            "no route to host")):
        return "Database host is not reachable from the cdc-pgdump container (DNS, network, port)."
    if "no pg_hba.conf entry" in s:
        return "Server pg_hba.conf rejects this client; allow the sidecar host or set an sslmode option."
    if "permission denied" in s:
        return "Role lacks privileges on some objects; narrow schemas/tables or grant pg_read_all_data."
    if "could not obtain lock" in s:
        return "A table lock was not granted within lock_wait_timeout_ms; retry off-peak."
    if "no matching tables were found" in s or "no matching schemas were found" in s:
        return "An include pattern matched nothing (patterns are case-sensitive when quoted)."
    if "schema" in s and "does not exist" in s:
        return ("A schema is missing on the target. Create it there, or dump by schemas instead of "
                "include_tables.")
    if "already exists" in s:
        return "Objects already exist on the target; use restore_clean + restore_if_exists or an empty database."
    return None


# ── job store ───────────────────────────────────────────────────────────────


def job_dir(job_id: str) -> Path:
    if not JOB_ID.match(job_id):
        raise BadRequest("invalid job id", 404)
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


def update_meta(job_id: str, **fields: Any) -> dict[str, Any]:
    with _lock:
        path = job_dir(job_id) / "job.json"
        meta = read_json(path)
        meta.update(fields)
        write_json(path, meta)
        return meta


def job_status(job_id: str) -> dict[str, Any]:
    # Read job.json under _lock: _send_worker pops _sending and appends to uploads in
    # one locked step, so reading the file first could see neither (send "vanishes").
    with _lock:
        meta = read_json(job_dir(job_id) / "job.json")
        rt = dict(_runtime.get(job_id) or {})
        sending = dict(_sending[job_id]) if job_id in _sending else None
    if not meta:
        return {"id": job_id, "status": "unknown"}
    meta.update(rt)
    if sending is not None:
        meta["uploads"] = list(meta.get("uploads") or []) + [sending]
        meta["sending"] = True
    art = meta.get("artifact") or {}
    if meta.get("status") in ACTIVE and art.get("path"):
        try:
            art["bytes"] = Path(art["path"]).stat().st_size
        except OSError:
            pass
    local_exists = bool(art.get("path")) and Path(art["path"]).is_file()
    if art:
        art = {k: v for k, v in art.items() if k != "path"}
        meta["artifact"] = art
    if local_exists and meta.get("status") not in ACTIVE:
        meta["download_path"] = f"/v1/jobs/{job_id}/file"
    meta["id"] = job_id
    return meta


def tail_log(path: Path, max_bytes: int = 256_000) -> str:
    if not path.is_file():
        return ""
    data = path.read_bytes()
    if len(data) > max_bytes:
        data = data[-max_bytes:]
    return data.decode("utf-8", errors="replace")


def active_jobs() -> list[str]:
    with _lock:
        return [j for j, rt in _runtime.items() if rt.get("status") in ACTIVE]


def free_mb() -> int:
    try:
        return shutil.disk_usage(WORK_ROOT).free // (1024 * 1024)
    except OSError:
        return -1


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


# ── execution ───────────────────────────────────────────────────────────────


def _run_step(job_id: str, phase: str, argv: list[str], env: dict[str, str], log_fh: Any,
              deadline: float | None) -> int:
    log_fh.write(f"\n--- {phase} at {utc_now()} ---\n{' '.join(argv)}\n".encode())
    log_fh.flush()
    # Check-and-spawn under the lock: stop_job either sees the flag first or finds the process.
    with _lock:
        if job_id in _stop_requested:
            return -signal.SIGTERM
        proc = subprocess.Popen(argv, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                cwd=str(job_dir(job_id)), start_new_session=True)
        _procs[job_id] = proc
        _runtime[job_id].update({"phase": phase, "pid": proc.pid})

    def pump() -> None:
        assert proc.stdout is not None
        for raw in proc.stdout:
            log_fh.write(raw)
            line = raw.decode("utf-8", errors="replace")
            if "dumping contents of table" in line or "processing data for table" in line:
                with _lock:
                    key = "tables_dumped" if phase == "dump" else "tables_restored"
                    _runtime[job_id][key] = _runtime[job_id].get(key, 0) + 1
            with _lock:
                _runtime[job_id]["last_message"] = line.strip()[:300]
        log_fh.flush()

    t = threading.Thread(target=pump, daemon=True)
    t.start()
    while True:
        try:
            code = proc.wait(timeout=1)
            break
        except subprocess.TimeoutExpired:
            if deadline and time.monotonic() > deadline:
                _killpg(proc)
                t.join(5)
                if proc.stdout is not None:
                    proc.stdout.close()
                raise TimeoutError(f"{phase} exceeded PGDUMP_TIMEOUT_SECONDS={TIMEOUT_SECONDS}")
    t.join(5)
    if proc.stdout is not None:
        proc.stdout.close()
    with _lock:
        _procs.pop(job_id, None)
    return code


def _killpg(proc: subprocess.Popen[Any]) -> None:
    try:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def prepare_target_schemas(job_id: str, artifact: Path, env: dict[str, str], log_fh: Any) -> None:
    """Create the schemas a table-filtered archive needs; pg_dump --table omits CREATE SCHEMA."""
    toc = subprocess.run([PG_RESTORE, "--list", str(artifact)], capture_output=True, text=True,
                         timeout=120, check=True).stdout
    schemas, skipped = toc_schemas(toc)
    log_fh.write(f"\n--- prepare at {utc_now()} ---\nensure target schemas: {schemas}\n".encode())
    if skipped:
        log_fh.write(f"not auto-created (not plain identifiers): {skipped}\n".encode())
    if schemas:
        out = subprocess.run([PSQL, "--no-password", "-v", "ON_ERROR_STOP=1", "-X", "-q", "-c",
                              create_schemas_sql(schemas)], env=env, capture_output=True, text=True,
                             timeout=120)
        log_fh.write((out.stdout + out.stderr).encode())
        if out.returncode != 0:
            raise RuntimeError(f"could not create target schemas {schemas}: {out.stderr.strip()[:300]}")
    log_fh.flush()


def upload_hint(error: str) -> str | None:
    e = error.lower()
    if "host key mismatch" in e:
        return ("The server's host key differs from the pinned fingerprint: the server was reinstalled "
                "or this is not the expected host. Verify, then update host_key_fingerprint.")
    if "authentication" in e or "authenticationexception" in e:
        return "SFTP login was rejected; check username and password/private key for this destination."
    if "private key" in e:
        return "The private key could not be loaded; paste the full key (BEGIN … END lines) and passphrase."
    if "permission denied" in e or "permissionerror" in e or "errno 13" in e:
        return "The SFTP user cannot write remote_dir; fix directory ownership/permissions or choose another path."
    if any(x in e for x in ("timed out", "timeout", "connection refused", "no route", "name or service",
                            "nodename nor servname", "unable to connect")):
        return "SFTP server not reachable from the cdc-pgdump container (DNS, firewall, port)."
    if "no space" in e or "quota" in e:
        return "The SFTP server ran out of space."
    return None


def upload_all(job_id: str, req: dict[str, Any], artifact: Path, digest: str, log_fh: Any) -> list[str]:
    """Upload to each destination in order; returns names of destinations that failed."""
    with _lock:
        _runtime[job_id]["phase"] = "upload"
        _runtime[job_id]["uploads"] = [dict(describe_destination(d), status="pending")
                                       for d in req["destinations"]]
    failed = []
    for i, dest in enumerate(req["destinations"]):
        if job_id in _stop_requested:
            break
        with _lock:
            _runtime[job_id]["uploads"][i]["status"] = "uploading"
        label = f"{dest['name']} ({dest['username']}@{dest['host']}:{dest['port']}:{dest['remote_dir']})"
        log_fh.write(f"\n--- upload at {utc_now()} ---\n{label}\n".encode())
        log_fh.flush()
        started = time.monotonic()

        def progress(sent: int, total: int, i: int = i) -> None:
            with _lock:
                _runtime[job_id]["uploads"][i]["bytes_sent"] = sent

        try:
            res = UPLOADER(dest, artifact, digest, progress, lambda: job_id in _stop_requested)
            res.update(status="uploaded", seconds=round(time.monotonic() - started, 1))
            log_fh.write(f"ok: {res.get('remote_path')} ({res.get('bytes')} bytes, host key "
                         f"{res.get('host_key_fingerprint')}"
                         f"{' pinned' if res.get('host_key_verified') else ' NOT pinned'})\n".encode())
        except UploadStopped:
            res = {"status": "stopped"}
            log_fh.write(b"stopped\n")
        except Exception as exc:  # noqa: BLE001 - one destination failing must not stop the others
            msg = str(exc) or type(exc).__name__
            res = {"status": "failed", "error": f"{type(exc).__name__}: {msg}"[:400]}
            hint = upload_hint(res["error"])
            if hint:
                res["hint"] = hint
            failed.append(dest["name"])
            log_fh.write(f"failed: {res['error']}\n".encode())
        log_fh.flush()
        with _lock:
            _runtime[job_id]["uploads"][i].update(res)
    return failed


def run_job(job_id: str, req: dict[str, Any], artifact: Path, passfile: Path | None) -> None:
    log_path = job_dir(job_id) / "pgdump.log"
    deadline = time.monotonic() + TIMEOUT_SECONDS if TIMEOUT_SECONDS > 0 else None
    status, error, exit_code, ignored = "failed", None, None, None
    upload_failed: list[str] = []
    try:
        update_meta(job_id, status="running", started_at=utc_now())
        with _lock:
            _runtime[job_id]["status"] = "running"
        with log_path.open("ab") as log_fh:
            env = libpq_env(dict(os.environ), req["source"], passfile, req["source_sslmode"])
            exit_code = _run_step(job_id, "dump", build_dump_argv(req, artifact), env, log_fh, deadline)
            if job_id in _stop_requested:
                status = "stopped"
            elif exit_code != 0:
                error = f"pg_dump exited with code {exit_code}"
            else:
                digest = sha256(artifact)
                update_meta(job_id, artifact={"path": str(artifact), "name": artifact.name,
                                              "format": req["format"], "bytes": artifact.stat().st_size,
                                              "sha256": digest})
                if req["destinations"]:
                    upload_failed = upload_all(job_id, req, artifact, digest, log_fh)
                if job_id in _stop_requested:
                    status = "stopped"
                elif req["mode"] == "dump":
                    status = "succeeded"
                else:
                    env = libpq_env(dict(os.environ), req["target"], passfile, req["target_sslmode"])
                    if req["include_tables"]:
                        prepare_target_schemas(job_id, artifact, env, log_fh)
                    exit_code = _run_step(job_id, "restore", build_restore_argv(req, artifact), env,
                                          log_fh, deadline)
                    tail = tail_log(log_path, 8000)
                    m = IGNORED_ERRORS.search(tail)
                    if job_id in _stop_requested:
                        status = "stopped"
                    elif exit_code == 0:
                        status = "succeeded"
                    elif m and not req["flags"]["restore_exit_on_error"]:
                        # pg_restore keeps going past object errors and reports the count with exit 1.
                        status, ignored = "completed_with_errors", int(m.group(1))
                        error = f"pg_restore ignored {ignored} error(s); see logs"
                    else:
                        error = f"pg_restore exited with code {exit_code}"
        if upload_failed and status in ("succeeded", "completed_with_errors"):
            msg = f"upload failed for: {', '.join(upload_failed)} (local file kept)"
            error = f"{error}; {msg}" if error else msg
            status = "completed_with_errors"
    except TimeoutError as exc:
        error = str(exc)
    except Exception as exc:  # noqa: BLE001 - surface any failure on the job
        error = f"{type(exc).__name__}: {exc}"
    finally:
        if passfile is not None:
            passfile.unlink(missing_ok=True)
        tail = tail_log(log_path, 8000)
        fields: dict[str, Any] = {"status": status, "finished_at": utc_now(), "exit_code": exit_code}
        with _lock:
            rt = _runtime.pop(job_id, {})
            _stop_requested.discard(job_id)
            _procs.pop(job_id, None)
        for key in ("tables_dumped", "tables_restored", "uploads"):
            if key in rt:
                fields[key] = rt[key]
        if (status == "succeeded" and req["destinations"] and not upload_failed
                and not req["keep_local"] and artifact.exists()):
            # Every destination has a verified copy: free the sidecar volume.
            artifact.unlink(missing_ok=True)
            meta_art = read_json(job_dir(job_id) / "job.json").get("artifact") or {}
            meta_art.update(local_deleted=True)
            fields["artifact"] = meta_art
        if ignored is not None:
            fields["errors_ignored"] = ignored
        if error:
            fields["error"] = error
            fe = first_error(tail)
            if fe:
                fields["first_error"] = fe
            hint = None if (upload_failed and not ignored) else hint_for(tail)
            if hint:
                fields["hint"] = hint
        if status in ("failed", "stopped") and req["mode"] == "dump":
            artifact.unlink(missing_ok=True)
            fields["artifact"] = None
        update_meta(job_id, **fields)


def start_job(body: dict[str, Any]) -> dict[str, Any]:
    req = validate_request(body)
    if MIN_FREE_MB > 0 and 0 <= free_mb() < MIN_FREE_MB:
        raise BadRequest(f"dump volume has {free_mb()} MB free; need {MIN_FREE_MB} MB", 507)
    with _lock:
        running = [rt for rt in _runtime.values() if rt.get("status") in ACTIVE]
        if len(running) >= MAX_CONCURRENT:
            raise BadRequest(f"{len(running)} dump job(s) already running (PGDUMP_MAX_CONCURRENT)", 429)
        if req["profile_id"] and any(rt.get("profile_id") == req["profile_id"] for rt in running):
            raise BadRequest("a job for this profile is already running", 409)
        job_id = str(uuid.uuid4())
        _runtime[job_id] = {"status": "starting", "profile_id": req["profile_id"]}

    work = job_dir(job_id)
    work.mkdir(parents=True, exist_ok=True)
    artifact = work / artifact_name(req, job_id)
    passfile = None
    content = pgpass_content([req["source"], req["target"]])
    if content:
        passfile = work / ".pgpass"
        fd = os.open(passfile, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(content)

    meta = {
        "id": job_id,
        "name": req["name"] or job_id,
        "profile_id": req["profile_id"],
        "mode": req["mode"],
        "status": "starting",
        "created_at": utc_now(),
        "source": describe(req["source"]),
        "target": describe(req["target"]) if req["target"] else None,
        "format": req["format"],
        "include_tables": req["include_tables"],
        "destinations": [describe_destination(d) for d in req["destinations"]],
        "keep_local": req["keep_local"],
        "argv": build_dump_argv(req, artifact),  # no secrets: credentials are env + passfile
        "log_path": str(work / "pgdump.log"),
    }
    if req["mode"] == "dump_restore":
        meta["restore_argv"] = build_restore_argv(req, artifact)
    write_json(work / "job.json", meta)
    threading.Thread(target=run_job, args=(job_id, req, artifact, passfile), daemon=True,
                     name=f"pgdump-{job_id[:8]}").start()
    return job_status(job_id)


def send_file(job_id: str, body: dict[str, Any]) -> dict[str, Any]:
    """Ad-hoc upload of a finished job's dump file to one SFTP server given in the request.

    Nothing about the server is stored except the credential-free result appended to the job's
    `uploads` (marked `adhoc`). The local file is never deleted by a send.
    """
    if not isinstance(body.get("destination"), dict):
        raise BadRequest("destination is required (one SFTP server)")
    dest = norm_destinations([body["destination"]])[0]
    work = job_dir(job_id)
    meta = read_json(work / "job.json")
    if not meta:
        raise BadRequest("job not found", 404)
    if meta.get("status") in ACTIVE or job_id in active_jobs():
        raise BadRequest(f"job is {meta.get('status')}; send the file after it finishes", 409)
    art = meta.get("artifact") or {}
    path = Path(art.get("path") or "")
    if not art or not path.is_file():
        raise BadRequest("the dump file is no longer on the dump volume; run the dump again", 410)
    with _lock:
        if job_id in _sending:
            raise BadRequest("this file is already being sent; wait for it or stop it", 409)
        _sending[job_id] = dict(describe_destination(dest), status="pending", adhoc=True, started_at=utc_now())
    # Snapshot before the worker starts: a fast send could otherwise finish first and the
    # response would not show the send it just accepted (sending=True, pending entry).
    accepted = job_status(job_id)
    threading.Thread(target=_send_worker, args=(job_id, dest, path, art.get("sha256") or ""), daemon=True,
                     name=f"pgdump-send-{job_id[:8]}").start()
    return accepted


def _send_worker(job_id: str, dest: dict[str, Any], path: Path, digest: str) -> None:
    res: dict[str, Any]
    started = time.monotonic()
    log_path = job_dir(job_id) / "pgdump.log"
    try:
        with log_path.open("ab") as log_fh:
            label = f"{dest['name']} ({dest['username']}@{dest['host']}:{dest['port']}:{dest['remote_dir']})"
            log_fh.write(f"\n--- send (ad hoc) at {utc_now()} ---\n{label}\n".encode())
            log_fh.flush()
            with _lock:
                _sending[job_id]["status"] = "uploading"

            def progress(sent: int, _total: int) -> None:
                with _lock:
                    if job_id in _sending:
                        _sending[job_id]["bytes_sent"] = sent

            try:
                res = UPLOADER(dest, path, digest or sha256(path), progress, lambda: job_id in _send_stop)
                res.update(status="uploaded")
                log_fh.write(f"ok: {res.get('remote_path')} ({res.get('bytes')} bytes, host key "
                             f"{res.get('host_key_fingerprint')}"
                             f"{' pinned' if res.get('host_key_verified') else ' NOT pinned'})\n".encode())
            except UploadStopped:
                res = {"status": "stopped"}
                log_fh.write(b"stopped\n")
            except Exception as exc:  # noqa: BLE001 - report on the upload entry
                msg = str(exc) or type(exc).__name__
                res = {"status": "failed", "error": f"{type(exc).__name__}: {msg}"[:400]}
                hint = upload_hint(res["error"])
                if hint:
                    res["hint"] = hint
                log_fh.write(f"failed: {res['error']}\n".encode())
    except Exception as exc:  # noqa: BLE001 - e.g. job dir removed underneath us
        res = {"status": "failed", "error": f"{type(exc).__name__}: {exc}"[:400]}
    res.update(seconds=round(time.monotonic() - started, 1), finished_at=utc_now())
    with _lock:
        entry = _sending.pop(job_id, {})
        _send_stop.discard(job_id)
        entry.update(res)
        meta_path = job_dir(job_id) / "job.json"
        meta = read_json(meta_path)
        if meta:
            meta["uploads"] = list(meta.get("uploads") or []) + [entry]
            write_json(meta_path, meta)


def stop_job(job_id: str) -> dict[str, Any]:
    with _lock:
        proc = _procs.get(job_id)
        if job_id in _sending:
            _send_stop.add(job_id)
        if job_id in _runtime:
            _stop_requested.add(job_id)
    if proc is not None and proc.poll() is None:
        _killpg(proc)
    return job_status(job_id)


def delete_job(job_id: str) -> bool:
    work = job_dir(job_id)
    if not work.is_dir():
        return False
    if job_id in active_jobs():
        raise BadRequest("job is running; stop it first", 409)
    with _lock:
        if job_id in _sending:
            raise BadRequest("the file is being sent; stop the send first", 409)
    shutil.rmtree(work, ignore_errors=True)
    return True


def list_jobs() -> list[dict[str, Any]]:
    sweep_expired()
    if not JOBS_DIR.is_dir():
        return []
    jobs = [job_status(c.name) for c in JOBS_DIR.iterdir() if c.is_dir() and JOB_ID.match(c.name)]
    jobs.sort(key=lambda j: j.get("created_at") or "", reverse=True)
    return jobs


def sweep_expired() -> None:
    if RETENTION_HOURS <= 0 or not JOBS_DIR.is_dir():
        return
    cutoff = datetime.now(timezone.utc) - timedelta(hours=RETENTION_HOURS)
    busy = set(active_jobs())
    with _lock:
        busy |= set(_sending)
    for child in JOBS_DIR.iterdir():
        if not child.is_dir() or child.name in busy:
            continue
        meta = read_json(child / "job.json")
        finished = meta.get("finished_at")
        try:
            if finished and datetime.fromisoformat(finished) < cutoff:
                shutil.rmtree(child, ignore_errors=True)
        except ValueError:
            continue


def recover_on_start() -> None:
    """A crash or restart orphans running jobs: mark them failed and remove leftover passfiles."""
    if not JOBS_DIR.is_dir():
        return
    for child in JOBS_DIR.iterdir():
        if not child.is_dir():
            continue
        (child / ".pgpass").unlink(missing_ok=True)
        meta = read_json(child / "job.json")
        if meta.get("status") in ACTIVE:
            meta.update(status="failed", error="sidecar restarted while the job was running",
                        finished_at=utc_now())
            write_json(child / "job.json", meta)


def tool_versions() -> dict[str, str]:
    out = {}
    for name, binary in (("pg_dump", PG_DUMP), ("pg_restore", PG_RESTORE)):
        try:
            out[name] = subprocess.check_output([binary, "--version"], text=True, timeout=5).strip()
        except (OSError, subprocess.SubprocessError):
            out[name] = "unavailable"
    return out


TEST_TIMEOUT_SECONDS = int(os.environ.get("PGDUMP_TEST_TIMEOUT_SECONDS", "20"))
MAX_TEST_SCHEMAS = 500
# One round trip; everything a profile editor needs to choose what to dump.
TEST_SQL = """select json_build_object(
  'server_version', current_setting('server_version'),
  'server_version_num', current_setting('server_version_num')::int,
  'current_user', current_user,
  'database_bytes', pg_database_size(current_database()),
  'ssl', coalesce((select ssl from pg_stat_ssl where pid = pg_backend_pid()), false),
  'schemas', coalesce((select json_agg(s order by s.name) from (
      select n.nspname as name,
             count(c.oid) filter (where c.relkind in ('r', 'p')) as tables,
             coalesce(sum(pg_total_relation_size(c.oid)) filter (where c.relkind in ('r', 'p', 'm')), 0)::bigint as bytes
        from pg_namespace n
        left join pg_class c on c.relnamespace = n.oid
       where n.nspname not in ('pg_catalog', 'information_schema') and n.nspname not like 'pg\\_%'
       group by n.nspname
       order by n.nspname
       limit {limit}) s), '[]'::json))""".replace("{limit}", str(MAX_TEST_SCHEMAS))


def major_of(version_text: str) -> int | None:
    """'pg_dump (PostgreSQL) 18.3 (Debian …)' → 18."""
    m = re.search(r"(\d+)(?:\.\d+)?", version_text.split(")", 1)[-1] if ")" in version_text else version_text)
    return int(m.group(1)) if m else None


def test_connection(body: dict[str, Any]) -> dict[str, Any]:
    """Connect from this container (where pg_dump runs) and describe the database.

    Failures are a normal answer (ok=false with error + hint), not an HTTP error, so the editor
    can show them beside the connection.
    """
    conn = norm_conn(body.get("connection"), "connection")
    sslmode = body.get("sslmode") or None
    if sslmode is not None and sslmode not in SSL_MODES:
        raise BadRequest(f"sslmode must be one of {', '.join(SSL_MODES)}")
    WORK_ROOT.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(prefix=".test-", dir=WORK_ROOT))
    try:
        passfile = None
        if conn["password"]:
            passfile = tmp / ".pgpass"
            fd = os.open(passfile, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, "w") as fh:
                fh.write(pgpass_content([conn]))
        env = libpq_env(dict(os.environ), conn, passfile, sslmode)
        env["PGCONNECT_TIMEOUT"] = str(min(TEST_TIMEOUT_SECONDS, 15))
        started = time.monotonic()
        try:
            proc = subprocess.run([PSQL, "-X", "-A", "-t", "-q", "-w", "-v", "ON_ERROR_STOP=1", "-c", TEST_SQL],
                                  env=env, capture_output=True, text=True, timeout=TEST_TIMEOUT_SECONDS)
        except subprocess.TimeoutExpired:
            return {"ok": False, "target": describe(conn),
                    "error": f"no answer within {TEST_TIMEOUT_SECONDS}s",
                    "hint": "Database host is not reachable from the cdc-pgdump container (DNS, network, port)."}
        latency_ms = int((time.monotonic() - started) * 1000)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    if proc.returncode != 0:
        err = "\n".join(line.strip() for line in proc.stderr.strip().splitlines() if line.strip())[:500]
        return {"ok": False, "target": describe(conn), "error": err or f"psql exited {proc.returncode}",
                "hint": hint_for(proc.stderr)}
    try:
        info = json.loads(proc.stdout.strip())  # json_agg spreads arrays over several lines
    except ValueError:
        return {"ok": False, "target": describe(conn), "error": "unexpected answer from server", "hint": None}

    client = tool_versions().get("pg_dump", "")
    client_major = major_of(client)
    server_major = int(info.get("server_version_num") or 0) // 10000 or None
    compatible = client_major is None or server_major is None or client_major >= server_major
    out = {"ok": True, "target": describe(conn), "latency_ms": latency_ms, "client_version": client,
           "client_major": client_major, "server_major": server_major, "compatible": compatible, **info}
    out.pop("server_version_num", None)
    if not compatible:
        out["hint"] = (f"Server is PostgreSQL {server_major} but pg_dump is {client_major}; "
                       f"rebuild cdc-pgdump with PG_MAJOR={server_major} or newer.")
    return out


CAPABILITY = {
    "service": "cdc-pgdump",
    "modes": list(MODES),
    "formats": list(FORMATS),
    "options": {
        "booleans": sorted(list(BOOL_DUMP) + list(BOOL_RESTORE)),
        "lists": sorted(LIST_DUMP),
        "integers": ["compress", "lock_wait_timeout_ms", "restore_jobs"],
        "strings": ["format", "source_sslmode", "target_sslmode"],
    },
    "connection_fields": ["host", "port", "database", "username", "password"],
    "include_tables": "Optional list of pg_dump table patterns (schema.table); empty means whole database",
    "destinations": {
        "max": MAX_DESTINATIONS,
        "fields": ["name", "host", "port", "username", "auth (key|password)", "password", "private_key",
                   "passphrase", "remote_dir", "host_key_fingerprint (SHA256:...)"],
        "behaviour": "upload <file>.part then rename; write <file>.sha256; local copy removed after all "
                     "uploads succeed unless options.keep_local",
        "adhoc_send": "POST /v1/jobs/{id}/send {destination} uploads a finished job's file to one more "
                      "server; the result is appended to uploads (adhoc: true); the local file is kept",
    },
}


# ── HTTP ────────────────────────────────────────────────────────────────────


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

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
            data = json.loads(raw.decode("utf-8"))
        except json.JSONDecodeError:
            raise BadRequest("body must be JSON") from None
        if not isinstance(data, dict):
            raise BadRequest("body must be a JSON object")
        return data

    def _route(self, method: str) -> None:
        if not self._auth_ok():
            self._json(401, {"error": "unauthorized"})
            return
        url = urlparse(self.path)
        parts = [p for p in url.path.split("/") if p]
        try:
            self._dispatch(method, parts, parse_qs(url.query))
        except BadRequest as exc:
            self._json(exc.code, {"error": str(exc)})
        except BrokenPipeError:
            pass
        except Exception as exc:  # noqa: BLE001
            self._json(500, {"error": f"{type(exc).__name__}: {exc}"})

    def _dispatch(self, method: str, parts: list[str], _query: dict[str, list[str]]) -> None:
        if method == "GET" and parts == ["health"]:
            self._json(200, {"status": "ok"})
        elif method == "GET" and parts in (["v1", "discover"], ["v1", "capability"]):
            out = dict(CAPABILITY)
            out.update(tool_versions())
            out.update({"free_mb": free_mb(), "max_concurrent": MAX_CONCURRENT,
                        "retention_hours": RETENTION_HOURS, "timeout_seconds": TIMEOUT_SECONDS})
            if parts[1] == "discover":
                out.update({"contract_version": "1", "tool_id": "pgdump", "sidecar_ready": True,
                            "orchestrator_contract": "/cdc/pgdump/contract"})
            self._json(200, out)
        elif parts == ["v1", "jobs"] and method == "GET":
            self._json(200, {"jobs": list_jobs()})
        elif parts == ["v1", "jobs"] and method == "POST":
            self._json(202, start_job(self._read_body()))
        elif parts == ["v1", "test-connection"] and method == "POST":
            self._json(200, test_connection(self._read_body()))
        elif len(parts) == 3 and parts[:2] == ["v1", "jobs"]:
            job_id = parts[2]
            if method == "GET":
                meta = job_status(job_id)
                self._json(404 if meta["status"] == "unknown" else 200,
                           {"error": "job not found"} if meta["status"] == "unknown" else meta)
            elif method == "DELETE":
                if delete_job(job_id):
                    self._json(200, {"id": job_id, "deleted": True})
                else:
                    self._json(404, {"error": "job not found"})
            else:
                self._json(405, {"error": "method not allowed"})
        elif len(parts) == 4 and parts[:2] == ["v1", "jobs"]:
            job_id, action = parts[2], parts[3]
            if action == "logs" and method == "GET":
                self._json(200, {"id": job_id, "log": tail_log(job_dir(job_id) / "pgdump.log")})
            elif action == "stop" and method == "POST":
                meta = job_status(job_id)
                if meta["status"] == "unknown":
                    self._json(404, {"error": "job not found"})
                else:
                    self._json(200, stop_job(job_id))
            elif action == "file" and method == "GET":
                self._send_file(job_id)
            elif action == "send" and method == "POST":
                self._json(202, send_file(job_id, self._read_body()))
            else:
                self._json(404, {"error": "not found"})
        else:
            self._json(404, {"error": "not found"})

    def _send_file(self, job_id: str) -> None:
        meta = read_json(job_dir(job_id) / "job.json")
        if not meta:
            self._json(404, {"error": "job not found"})
            return
        art = meta.get("artifact") or {}
        path = Path(art.get("path") or "")
        if meta.get("status") in ACTIVE or job_id in active_jobs():
            self._json(409, {"error": f"job is {meta.get('status')}; file not ready"})
            return
        if not art or not path.is_file():
            if art.get("local_deleted"):
                where = ", ".join(f"{u.get('name')}:{u.get('remote_path')}" for u in meta.get("uploads") or []
                                  if u.get("status") == "uploaded")
                self._json(410, {"error": f"dump file was removed after upload; copies at {where}"})
            else:
                self._json(410, {"error": "no dump file for this job"})
            return
        size = path.stat().st_size
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(size))
        self.send_header("Content-Disposition", f'attachment; filename="{path.name}"')
        if art.get("sha256"):
            self.send_header("X-Dump-Sha256", art["sha256"])
        self.end_headers()
        with path.open("rb") as fh:
            shutil.copyfileobj(fh, self.wfile, 1 << 20)

    def do_GET(self) -> None:
        self._route("GET")

    def do_POST(self) -> None:
        self._route("POST")

    def do_DELETE(self) -> None:
        self._route("DELETE")


def main() -> None:
    JOBS_DIR.mkdir(parents=True, exist_ok=True)
    recover_on_start()
    sweep_expired()
    server = ThreadingHTTPServer((HOST, PORT), Handler)
    print(f"cdc-pgdump listening on {HOST}:{PORT} (work_root={WORK_ROOT})", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
