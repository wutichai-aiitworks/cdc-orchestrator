"""Connection tests shared by the Copy & Sync sidecars. Each test runs inside the sidecar, so it
proves the same network path and login the tool will use. Results are plain answers:
{ok, target, facts[], warning?, error?, hint?} — a failed connection is not an HTTP error.

Secrets never reach argv: Postgres uses a 0600 passfile in a temporary directory; other tools
read the URI from the environment.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any

from jobstore import fmt_bytes, plural, probe, write_secret

SSL_MODES = ("disable", "allow", "prefer", "require", "verify-ca", "verify-full")
HOST_RE = re.compile(r"^[A-Za-z0-9._:\-\[\]]{1,253}$")


class BadRequest(Exception):
    pass


def hint_for(text: str) -> str | None:
    s = (text or "").lower()
    if "password authentication failed" in s or "access denied" in s or "authentication failed" in s or "wrongpass" in s or "noauth" in s or "no password supplied" in s or "login failed" in s:
        return "The server rejected the login. Check the username and password."
    if any(x in s for x in ("could not translate host name", "name or service not known", "no such host", "getaddrinfo", "nodename nor servname")):
        return "The host name does not resolve from the tool's container. Use a name or IP it can reach (for this machine: host.docker.internal)."
    if any(x in s for x in ("connection refused", "timed out", "timeout", "no route to host", "server selection", "unreachable", "econnrefused")):
        return "The server did not accept the connection from the tool's container. Check host, port and firewall."
    if "no pg_hba.conf entry" in s:
        return "pg_hba.conf on the server rejects this client. Allow the tool's address or change the TLS setting."
    if "does not exist" in s and "database" in s:
        return "The database does not exist on that server. Check its name."
    if any(x in s for x in ("ssl", "tls", "certificate", "wrong version number")):
        return "The TLS setting does not match the server. Turn TLS on or off to match it."
    if "permission denied" in s or "not authorized" in s:
        return "The user may log in but lacks a privilege the tool needs."
    return None


def norm_conn(conn: Any, role: str = "connection", default_port: str = "", need_database: bool = True, need_user: bool = True) -> dict[str, str]:
    if not isinstance(conn, dict) or not conn:
        raise BadRequest(f"{role} is required")
    host = str(conn.get("host") or "").strip()
    if not host or not HOST_RE.match(host):
        raise BadRequest(f"{role}.host is required and must be a hostname or IP")
    port = str(conn.get("port") or default_port).strip()
    if port and (not port.isdigit() or not 1 <= int(port) <= 65535):
        raise BadRequest(f"{role}.port must be 1-65535")
    database = str(conn.get("database") or "").strip()
    if need_database and not database:
        raise BadRequest(f"{role}.database is required")
    user = str(conn.get("username") or conn.get("user") or "").strip()
    if need_user and not user:
        raise BadRequest(f"{role}.username is required")
    return {"type": str(conn.get("type") or ""), "host": host, "port": port, "database": database, "username": user, "password": str(conn.get("password") or "")}


def describe(c: dict[str, str]) -> str:
    who = f"{c['username']}@" if c.get("username") else ""
    db = f"/{c['database']}" if c.get("database") else ""
    return f"{who}{c['host']}:{c['port']}{db}"


# ── PostgreSQL ──────────────────────────────────────────────────────────────


def pgpass_escape(value: str) -> str:
    return value.replace("\\", "\\\\").replace(":", "\\:")


def pgpass_content(conns: list[dict[str, str]]) -> str:
    """Exact host:port:*:user lines, so one server's password is never offered to another."""
    lines = []
    for c in conns:
        if c and c.get("password"):
            lines.append(":".join([pgpass_escape(c["host"]), pgpass_escape(c["port"] or "5432"), "*", pgpass_escape(c["username"]), pgpass_escape(c["password"])]))
    return "\n".join(lines) + ("\n" if lines else "")


def pg_env(base: dict[str, str], passfile: Path | None, conn: dict[str, str] | None = None, sslmode: str | None = None) -> dict[str, str]:
    """Child env without inherited PG* variables (nothing leaks between runs)."""
    env = {k: v for k, v in base.items() if not k.startswith("PG")}
    if conn:
        env.update({"PGHOST": conn["host"], "PGPORT": conn["port"] or "5432", "PGUSER": conn["username"], "PGDATABASE": conn["database"]})
    if passfile is not None:
        env["PGPASSFILE"] = str(passfile)
    if sslmode:
        env["PGSSLMODE"] = sslmode
    env["PGAPPNAME"] = env.get("PGAPPNAME", "cdc-sidecar")
    return env


PG_SQL = """select json_build_object(
  'server_version', current_setting('server_version'),
  'server_version_num', current_setting('server_version_num')::int,
  'database_bytes', pg_database_size(current_database()),
  'schemas', (select count(*) from pg_namespace where nspname not in ('pg_catalog', 'information_schema') and nspname not like 'pg\\_%'),
  'tables', (select count(*) from pg_class c join pg_namespace n on n.oid = c.relnamespace
             where c.relkind in ('r', 'p') and n.nspname not in ('pg_catalog', 'information_schema')))"""


def client_major(binary: str) -> int | None:
    try:
        out = subprocess.check_output([binary, "--version"], text=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        return None
    m = re.search(r"\)\s*(\d+)", out) or re.search(r"(\d+)\.", out)
    return int(m.group(1)) if m else None


def pg_test(conn: Any, *, tmp_root: Path, sslmode: str | None = None, psql: str = "psql", dump_client: str | None = "pg_dump", timeout: int = 20) -> dict[str, Any]:
    c = norm_conn(conn, default_port="5432")
    if sslmode is not None and sslmode not in SSL_MODES:
        raise BadRequest(f"sslmode must be one of {', '.join(SSL_MODES)}")
    tmp_root.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(prefix=".test-", dir=tmp_root))
    try:
        passfile = write_secret(tmp / ".pgpass", pgpass_content([c])) if c["password"] else None
        env = pg_env(dict(os.environ), passfile, c, sslmode)
        env["PGCONNECT_TIMEOUT"] = str(min(timeout, 15))
        res = probe([psql, "-X", "-A", "-t", "-q", "-w", "-v", "ON_ERROR_STOP=1", "-c", PG_SQL], env=env, timeout=timeout)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    if not res["ok"]:
        return {"ok": False, "target": describe(c), "error": res["error"], "hint": hint_for(res["error"])}
    try:
        info = json.loads(res["stdout"].strip())
    except ValueError:
        return {"ok": False, "target": describe(c), "error": "unexpected answer from the server", "hint": None}
    server = int(info.get("server_version_num") or 0) // 10000 or None
    out: dict[str, Any] = {
        "ok": True,
        "target": describe(c),
        "latency_ms": res["latency_ms"],
        "server_version": info.get("server_version"),
        "server_major": server,
        "facts": [
            f"PostgreSQL {str(info.get('server_version') or '').split(' ')[0]}",
            fmt_bytes(info.get("database_bytes")),
            plural(info.get("tables", 0), "table"),
            f"{res['latency_ms']} ms",
        ],
    }
    if dump_client:
        client = client_major(dump_client)
        out["client_major"] = client
        if client and server and client < server:
            out["warning"] = f"The server is PostgreSQL {server} but this tool's client is {client}; runs will fail until the image is rebuilt with a newer client."
    return out
