#!/usr/bin/env python3
"""HTTP sidecar for Postgres copy (pgcopydb clone). Contract: sidecar_common/http_api.py.

One job per run (history per transfer). Passwords never reach argv, job.json or the log: the
URIs carry no password and libpq reads it from the run's 0600 .pgpass, deleted when the run ends.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any
from urllib.parse import quote

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "sidecar_common"))  # local tests

from http_api import Tool, serve  # noqa: E402
from jobstore import JobStore, utc_now, write_secret  # noqa: E402
from probes import BadRequest, norm_conn, pg_env, pg_test, pgpass_content  # noqa: E402

WORK_ROOT = Path(os.environ.get("PGCOPY_WORK_ROOT", "/var/lib/pgcopydb"))
JOBS_DIR = WORK_ROOT / "jobs"
API_TOKEN = os.environ.get("PGCOPY_API_TOKEN", "").strip()
HOST = os.environ.get("PGCOPY_API_HOST", "0.0.0.0")
PORT = int(os.environ.get("PGCOPY_API_PORT", "8085"))
PGCOPYDB = os.environ.get("PGCOPY_BIN", "pgcopydb")
RETAIN = int(os.environ.get("PGCOPY_RETAIN_RUNS", "10"))

STORE = JobStore(JOBS_DIR, "clone.log", retain_per_profile=RETAIN)


def pg_uri(conn: dict[str, Any]) -> str:
    """postgres://user@host:port/db — no password (it comes from PGPASSFILE)."""
    user = quote(str(conn.get("username") or conn.get("user") or ""), safe="")
    host = conn.get("host") or "localhost"
    port = conn.get("port") or "5432"
    database = quote(str(conn.get("database") or "postgres"), safe="")
    return f"postgresql://{user + '@' if user else ''}{host}:{port}/{database}"


def write_filters(work: Path, include_tables: list[str]) -> Path | None:
    if not include_tables:
        return None
    path = work / "filters.ini"
    # pgcopydb ignores unknown sections, so "[include]" silently copied every table.
    lines = ["[include-only-table]"]
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
    argv = [PGCOPYDB, "clone", "--source", pg_uri(source), "--target", pg_uri(target), "--dir", str(work / "clone")]
    filters = write_filters(work, body.get("include_tables") or [])
    if filters:
        argv.extend(["--filters", str(filters)])
    for key, flag in BOOL_FLAGS.items():
        if options.get(key):
            argv.append(flag)
    for key, flag in {**INT_OPTS, **STR_OPTS}.items():
        val = options.get(key)
        if val is not None and str(val).strip() != "":
            argv.extend([flag, str(val)])
    extra_filters = options.get("filters_file")
    if extra_filters and str(extra_filters).strip():
        argv.extend(["--filters", str(extra_filters)])
    return argv


def take_previous_state(work: Path, profile_id: str, options: dict[str, Any]) -> str | None:
    """"Resume" continues the transfer's previous run: move its work dir into this run.
    Without resume every run starts clean (state from a failed run broke the next one)."""
    if not options.get("resume"):
        return None
    for meta in STORE.list(profile_id):
        prev = JOBS_DIR / meta["id"] / "clone"
        if meta["id"] != work.name and prev.is_dir():
            shutil.move(str(prev), str(work / "clone"))
            return meta["id"]
    return None


def cleanup_secrets(work: Path) -> None:
    (work / ".pgpass").unlink(missing_ok=True)


def start_job(body: dict[str, Any]) -> dict[str, Any]:
    source = norm_conn(body.get("source"), "source", "5432")
    target = norm_conn(body.get("target"), "target", "5432")
    options = body.get("options") or {}
    job_id, work, meta = STORE.new_job(body)
    try:
        resumed_from = take_previous_state(work, meta["profile_id"], options)
        argv = build_clone_argv(body, work)
        passfile = write_secret(work / ".pgpass", pgpass_content([source, target]))
        env = pg_env(dict(os.environ), passfile)
        env["PGAPPNAME"] = "cdc-pgcopydb"
        STORE.update(job_id, argv=argv, source=f"{source['username']}@{source['host']}:{source['port']}/{source['database']}",
                     target=f"{target['username']}@{target['host']}:{target['port']}/{target['database']}",
                     resumed_from=resumed_from)
        header = f"\n--- pgcopydb job {job_id} at {utc_now()} ---\n{' '.join(argv)}\n"
        return STORE.spawn(job_id, argv, env=env, header=header, on_exit=lambda jid, code: cleanup_secrets(work))
    except Exception as exc:
        cleanup_secrets(work)
        STORE.fail_start(job_id, f"{type(exc).__name__}: {exc}")
        raise


def test_connection(body: dict[str, Any]) -> dict[str, Any]:
    # Both ends are PostgreSQL; pg_dump in this image must be >= the source's major version.
    side = body.get("side") or "source"
    return pg_test(body.get("connection"), tmp_root=WORK_ROOT, sslmode=(body.get("options") or {}).get("sslmode"),
                   dump_client="pg_dump" if side == "source" else None)


def versions() -> dict[str, Any]:
    out: dict[str, Any] = {}
    for key, argv in (("pgcopydb_version", [PGCOPYDB, "--version"]), ("pg_dump", ["pg_dump", "--version"])):
        try:
            out[key] = subprocess.check_output(argv, text=True, stderr=subprocess.STDOUT, timeout=10).strip()
        except (OSError, subprocess.SubprocessError):
            out[key] = "unknown"
    return out


def capability() -> dict[str, Any]:
    return {
        "pgcopydb_command": "clone",
        "options": {"booleans": sorted(BOOL_FLAGS), "integers": sorted(INT_OPTS), "strings": sorted(STR_OPTS)},
        "connection_fields": ["host", "port", "database", "username", "password"],
        "include_tables": "Optional list of schema.table entries; empty means entire database",
        "retain_runs_per_profile": RETAIN,
        **versions(),
    }


TOOL = Tool(tool_id="pgcopy", service="cdc-pgcopydb", store=STORE, capability=capability, start=start_job,
            test=test_connection, api_token=API_TOKEN)

if __name__ == "__main__":
    STORE.recover_on_start(cleanup_secrets)
    serve(TOOL, HOST, PORT)
