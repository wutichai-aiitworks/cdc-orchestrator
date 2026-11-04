#!/usr/bin/env python3
"""HTTP sidecar for Mongo copy (mongodump → mongorestore). Contract: sidecar_common/http_api.py.

One job per run (history per transfer). Connection URIs (with passwords) go to 0600 --config
files, never argv; they and the dump archive are deleted as soon as the run ends.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any
from urllib.parse import quote

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "sidecar_common"))  # local tests

from http_api import Tool, serve  # noqa: E402
from jobstore import JobStore, plural, probe, utc_now, write_secret  # noqa: E402
from probes import BadRequest, describe, hint_for, norm_conn  # noqa: E402

WORK_ROOT = Path(os.environ.get("MONGOCOPY_WORK_ROOT", "/var/lib/mongocopy"))
JOBS_DIR = WORK_ROOT / "jobs"
API_TOKEN = os.environ.get("MONGOCOPY_API_TOKEN", "").strip()
HOST = os.environ.get("MONGOCOPY_API_HOST", "0.0.0.0")
PORT = int(os.environ.get("MONGOCOPY_API_PORT", "8088"))
MONGOSH = os.environ.get("MONGOCOPY_MONGOSH", "mongosh")
RETAIN = int(os.environ.get("MONGOCOPY_RETAIN_RUNS", "10"))

STORE = JobStore(JOBS_DIR, "mongocopy.log", retain_per_profile=RETAIN)


def mongo_uri(conn: dict[str, Any], options: dict[str, Any], with_database: bool = True) -> str:
    user = quote(str(conn.get("username") or conn.get("user") or ""), safe="")
    password = quote(str(conn.get("password") or ""), safe="")
    host = conn.get("host") or "localhost"
    port = str(conn.get("port") or "27017")
    database = str(conn.get("database") or "")
    auth_source = str(options.get("auth_source") or conn.get("schema") or "").strip()
    auth = ""
    if user and password:
        auth = f"{user}:{password}@"
    elif user:
        auth = f"{user}@"
    # mongorestore treats a database in the URI path as a namespace filter, so restores
    # put none there (namespaces come from --nsInclude/--nsFrom). The path also chose the
    # authentication database; keep that by naming it explicitly.
    if not with_database and database and user and not auth_source:
        auth_source = database
    path = f"/{database}" if database and with_database else "/"
    query = f"?authSource={quote(auth_source)}" if auth_source else ""
    return f"mongodb://{auth}{host}:{port}{path}{query}"


def ns_filters(body: dict[str, Any]) -> list[str]:
    source = body.get("source") or {}
    db = str(source.get("database") or "").strip()
    items = [str(item).strip() for item in (body.get("include_collections") or []) if str(item).strip()]
    out: list[str] = []
    for item in items:
        if "." in item:
            out.append(item)
        elif db:
            out.append(f"{db}.{item}")
    return out


def uri_config(path: Path, uri: str) -> Path:
    """A tools --config file (YAML); a JSON string is a valid YAML double-quoted scalar."""
    return write_secret(path, f"uri: {json.dumps(uri)}\n")


def build_commands(work: Path, body: dict[str, Any]) -> tuple[list[str], list[str]]:
    source = body.get("source") or {}
    target = body.get("target") or {}
    options = body.get("options") or {}
    archive = work / "dump.archive.gz"
    filters = ns_filters(body)
    src_db = str(source.get("database") or "").strip()
    tgt_db = str(target.get("database") or "").strip()

    src_cfg = uri_config(work / "source.yaml", mongo_uri(source, options))
    # mongorestore treats a database in the URI as a filter, so the target URI names none.
    tgt_cfg = uri_config(work / "target.yaml", mongo_uri(target, options, with_database=False))
    dump = ["mongodump", f"--config={src_cfg}", f"--archive={archive}"]
    restore = ["mongorestore", f"--config={tgt_cfg}", f"--archive={archive}"]
    if options.get("gzip", True):
        dump.append("--gzip")
        restore.append("--gzip")
    if options.get("drop"):
        restore.append("--drop")
    for ns in filters:
        dump.extend(["--nsInclude", ns])
        restore.extend(["--nsInclude", ns])
    if src_db and tgt_db and src_db != tgt_db and not filters:
        restore.extend([f"--nsInclude={src_db}.*", f"--nsFrom={src_db}.*", f"--nsTo={tgt_db}.*"])
    elif src_db and not filters:
        restore.extend(["--nsInclude", f"{src_db}.*"])
    return dump, restore


def write_run_script(work: Path, body: dict[str, Any]) -> Path:
    dump, restore = build_commands(work, body)

    def sh_join(parts: list[str]) -> str:
        return " ".join("'" + p.replace("'", "'\\''") + "'" for p in parts)

    script = work / "run.sh"
    script.write_text("#!/bin/sh\nset -eu\n" + sh_join(dump) + "\n" + sh_join(restore) + "\n", encoding="utf-8")
    script.chmod(0o700)
    return script


def cleanup(work: Path) -> None:
    for name in ("source.yaml", "target.yaml", "dump.archive.gz"):
        (work / name).unlink(missing_ok=True)


def start_job(body: dict[str, Any]) -> dict[str, Any]:
    job_id, work, meta = STORE.new_job(body)
    try:
        script = write_run_script(work, body)
        STORE.update(job_id, argv=["sh", "run.sh"])
        header = f"\n--- mongocopy job {job_id} at {utc_now()} ---\n"
        return STORE.spawn(job_id, ["sh", str(script)], header=header, on_exit=lambda jid, code: cleanup(work))
    except Exception as exc:
        cleanup(work)
        STORE.fail_start(job_id, f"{type(exc).__name__}: {exc}")
        raise


# One round trip in mongosh; the URI (with password) comes from the environment, not argv.
PROBE_JS = """
const c = connect(process.env.MONGO_PROBE_URI);
const hello = c.runCommand({ hello: 1 });
let dbs = null, bytes = null;
try { const l = c.adminCommand({ listDatabases: 1, nameOnly: false }); dbs = l.databases.length; bytes = l.totalSize; } catch (e) {}
let colls = null;
try { colls = c.getCollectionNames().length; } catch (e) {}
print(JSON.stringify({ version: c.version(), primary: hello.isWritablePrimary === true, setName: hello.setName || null, dbs, bytes, colls, db: c.getName() }));
"""


def test_connection(body: dict[str, Any]) -> dict[str, Any]:
    conn = norm_conn(body.get("connection"), default_port="27017", need_database=False, need_user=False)
    options = dict(body.get("options") or {})
    uri = mongo_uri(conn, options)
    uri += ("&" if "?" in uri else "?") + "serverSelectionTimeoutMS=8000&connectTimeoutMS=8000"
    env = {**os.environ, "MONGO_PROBE_URI": uri}
    res = probe([MONGOSH, "--nodb", "--quiet", "--norc", "--eval", PROBE_JS], env=env, timeout=25)
    if not res["ok"]:
        return {"ok": False, "target": describe(conn), "error": res["error"], "hint": hint_for(res["error"])}
    try:
        info = json.loads(res["stdout"].strip().splitlines()[-1])
    except (ValueError, IndexError):
        return {"ok": False, "target": describe(conn), "error": "unexpected answer from mongosh", "hint": None}
    facts = [f"MongoDB {info.get('version')}", f"{res['latency_ms']} ms"]
    if info.get("setName"):
        facts.insert(1, f"replica set {info['setName']}{'' if info.get('primary') else ', secondary'}")
    if conn["database"] and info.get("colls") is not None:
        facts.insert(1, f"{plural(info['colls'], 'collection')} in {conn['database']}")
    elif info.get("dbs") is not None:
        facts.insert(1, plural(info["dbs"], "database"))
    out: dict[str, Any] = {"ok": True, "target": describe(conn), "latency_ms": res["latency_ms"], "server_version": info.get("version"), "facts": facts}
    if (body.get("side") or "source") == "target" and info.get("setName") and not info.get("primary"):
        out["warning"] = "This server is a secondary; restores must go to the primary."
    return out


def capability() -> dict[str, Any]:
    vers = {}
    for key, argv in (("mongodump_version", ["mongodump", "--version"]), ("mongorestore_version", ["mongorestore", "--version"])):
        try:
            vers[key] = subprocess.check_output(argv, text=True, timeout=10).splitlines()[0].strip()
        except (OSError, subprocess.SubprocessError, IndexError):
            vers[key] = "unknown"
    return {
        "connection_fields": ["host", "port", "database", "username", "password"],
        "include_collections": "Optional list of collection or database.collection",
        "retain_runs_per_profile": RETAIN,
        **vers,
    }


TOOL = Tool(tool_id="mongocopy", service="cdc-mongocopy", store=STORE, capability=capability, start=start_job,
            test=test_connection, api_token=API_TOKEN)

if __name__ == "__main__":
    STORE.recover_on_start(cleanup)
    serve(TOOL, HOST, PORT)
