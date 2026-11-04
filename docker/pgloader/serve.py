#!/usr/bin/env python3
"""HTTP sidecar for Load to Postgres (pgloader). Contract: sidecar_common/http_api.py.

One job per run (history per transfer). The load file carries the connection URIs, so it is a
0600 file deleted as soon as the run ends; pgloader's own log prints connections without passwords.
"""

from __future__ import annotations

import csv
import os
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any
from urllib.parse import quote

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "sidecar_common"))  # local tests

from http_api import Tool, serve  # noqa: E402
from jobstore import JobStore, fmt_bytes, plural, probe, utc_now, write_secret  # noqa: E402
from probes import BadRequest, describe, hint_for, norm_conn, pg_test  # noqa: E402

WORK_ROOT = Path(os.environ.get("PGLOADER_WORK_ROOT", "/var/lib/pgloader"))
JOBS_DIR = WORK_ROOT / "jobs"
API_TOKEN = os.environ.get("PGLOADER_API_TOKEN", "").strip()
HOST = os.environ.get("PGLOADER_API_HOST", "0.0.0.0")
PORT = int(os.environ.get("PGLOADER_API_PORT", "8087"))
PGLOADER = os.environ.get("PGLOADER_BIN", "pgloader")
RETAIN = int(os.environ.get("PGLOADER_RETAIN_RUNS", "10"))

STORE = JobStore(JOBS_DIR, "pgloader.log", retain_per_profile=RETAIN)

SOURCE_ALIASES = {
    "mysql": "mysql",
    "mariadb": "mysql",
    "mssql": "mssql",
    "sqlserver": "mssql",
    "sqlite": "sqlite",
    "csv": "csv",
}


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

    # The load file holds the connection URIs (with passwords): 0600, deleted when the run ends.
    return write_secret(work / "load.load", "\n".join(lines) + "\n")



def cleanup_secrets(work: Path) -> None:
    (work / "load.load").unlink(missing_ok=True)


def start_job(body: dict[str, Any]) -> dict[str, Any]:
    if not conn_type(body.get("source") or {}):
        raise BadRequest("source.type is required")
    job_id, work, meta = STORE.new_job(body)
    try:
        load_file = write_load_file(work, body)
    except ValueError as exc:
        STORE.fail_start(job_id, str(exc))
        raise BadRequest(str(exc)) from None
    argv = [PGLOADER, str(load_file)]
    STORE.update(job_id, argv=[PGLOADER, "load.load"], source_type=conn_type(body.get("source") or {}))
    header = f"\n--- pgloader job {job_id} at {utc_now()} ---\n"
    try:
        return STORE.spawn(job_id, argv, header=header, on_exit=lambda jid, code: cleanup_secrets(work))
    except Exception as exc:
        cleanup_secrets(work)
        STORE.fail_start(job_id, f"{type(exc).__name__}: {exc}")
        raise


# ── connection tests ────────────────────────────────────────────────────────


def test_file(conn: dict[str, Any], kind: str, options: dict[str, Any]) -> dict[str, Any]:
    path = Path(file_path(conn))
    if not str(path):
        raise BadRequest("a file path is required")
    target = str(path)
    if not path.is_file():
        return {"ok": False, "target": target, "error": f"{path} does not exist inside the pgloader container",
                "hint": "Put the file on a volume mounted into cdc-pgloader and use its path inside the container."}
    if not os.access(path, os.R_OK):
        return {"ok": False, "target": target, "error": f"{path} is not readable", "hint": "Make the file readable by the sidecar's user."}
    size = fmt_bytes(path.stat().st_size)
    if kind == "sqlite":
        try:
            db = sqlite3.connect(f"file:{path}?mode=ro", uri=True, timeout=5)
            try:
                tables = db.execute("select count(*) from sqlite_master where type = 'table' and name not like 'sqlite_%'").fetchone()[0]
            finally:
                db.close()
        except sqlite3.DatabaseError as exc:
            return {"ok": False, "target": target, "error": str(exc), "hint": "The file is not a SQLite database."}
        return {"ok": True, "target": target, "facts": [f"SQLite file, {size}", plural(tables, "table")]}
    delimiter = str(options.get("csv_delimiter") or ",")[:1]
    with path.open(newline="", encoding="utf-8", errors="replace") as fh:
        rows = []
        for row in csv.reader(fh, delimiter=delimiter):
            rows.append(row)
            if len(rows) >= 2:
                break
    if not rows:
        return {"ok": False, "target": target, "error": "the file is empty", "hint": None}
    cols = len(rows[0])
    out: dict[str, Any] = {"ok": True, "target": target, "facts": [f"CSV file, {size}", plural(cols, "column")]}
    if cols == 1 and delimiter not in rows[0][0] and any(d in rows[0][0] for d in ",;\t|"):
        out["warning"] = f"Only one column is found with the separator “{delimiter}”. The file may use a different separator."
    elif rows:
        out["facts"].append("first line: " + ", ".join(rows[0][:6]) + (" …" if cols > 6 else ""))
    return out


def test_database_source(conn: dict[str, Any], kind: str) -> dict[str, Any]:
    """pgloader --dry-run opens each connection with the same driver a load uses; the target is
    a dummy, and the source's own Success/Failed line is the answer."""
    default_port = "3306" if kind == "mysql" else "1433"
    c = norm_conn(conn, default_port=default_port)
    scheme = "mysql" if kind == "mysql" else "mssql"
    with tempfile.TemporaryDirectory(dir=WORK_ROOT) as tmp:
        load = write_secret(Path(tmp) / "probe.load",
                            f"LOAD DATABASE FROM {sql_uri(scheme, c, default_port)} INTO postgresql://probe@127.0.0.1:1/probe;\n")
        res = probe([PGLOADER, "--dry-run", str(load)], timeout=40, cwd=tmp)
    text = (res.get("stdout") or "") + "\n" + (res.get("error") or "")
    marker = "#<MYSQL-CONNECTION" if kind == "mysql" else "#<MSSQL-CONNECTION"
    for line in text.splitlines():
        if marker in line and "Success, opened" in line:
            return {"ok": True, "target": describe(c), "latency_ms": res.get("latency_ms"),
                    "facts": [("MySQL" if kind == "mysql" else "SQL Server") + " login accepted", f"{res.get('latency_ms')} ms"]}
        if marker in line and "Failed to connect" in line:
            reason = line.split(">: ", 1)[-1].strip()
            return {"ok": False, "target": describe(c), "error": reason[:600], "hint": hint_for(reason)}
    return {"ok": False, "target": describe(c), "error": (res.get("error") or text).strip()[-600:] or "no answer from pgloader", "hint": hint_for(text)}


def test_connection(body: dict[str, Any]) -> dict[str, Any]:
    side = body.get("side") or "source"
    conn = body.get("connection") or {}
    options = body.get("options") or {}
    WORK_ROOT.mkdir(parents=True, exist_ok=True)
    if side == "target":
        return pg_test(conn, tmp_root=WORK_ROOT, dump_client=None)
    kind = conn_type(conn)
    if kind in ("csv", "sqlite"):
        return test_file(conn, kind, options)
    if kind in ("mysql", "mssql"):
        return test_database_source(conn, kind)
    raise BadRequest(f"unsupported source type: {kind or '(empty)'}")


def capability() -> dict[str, Any]:
    try:
        ver = subprocess.check_output([PGLOADER, "--version"], text=True, timeout=10).strip()
    except (OSError, subprocess.SubprocessError):
        ver = "unknown"
    return {
        "source_types": sorted(set(SOURCE_ALIASES.values())),
        "connection_fields": ["type", "host", "port", "database", "username", "password"],
        "include_tables": "Optional table name patterns (INCLUDING ONLY TABLE NAMES MATCHING)",
        "retain_runs_per_profile": RETAIN,
        "pgloader_version": ver,
    }


TOOL = Tool(tool_id="pgloader", service="cdc-pgloader", store=STORE, capability=capability, start=start_job,
            test=test_connection, api_token=API_TOKEN)

if __name__ == "__main__":
    STORE.recover_on_start(cleanup_secrets)
    serve(TOOL, HOST, PORT)
