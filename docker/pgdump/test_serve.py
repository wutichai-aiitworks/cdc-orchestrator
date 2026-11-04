"""Unit tests for the cdc-pgdump sidecar.  Run: cd docker/pgdump && python3 -m unittest -v"""

from __future__ import annotations

import json
import os
import stat
import tempfile
import time
import unittest
from pathlib import Path

import serve

SRC = {"type": "postgres", "host": "src-db", "port": "5432", "database": "shop", "username": "reader",
       "password": "s:ec\\ret"}
TGT = {"type": "postgresql", "host": "tgt-db", "port": "6543", "database": "replica", "username": "writer",
       "password": "tgt-pw"}


class ValidateTest(unittest.TestCase):
    def test_defaults_dump_mode_custom_format(self):
        req = serve.validate_request({"source": SRC})
        self.assertEqual(req["mode"], "dump")
        self.assertEqual(req["format"], "custom")
        self.assertIsNone(req["target"])

    def test_dump_restore_requires_target_and_custom_format(self):
        with self.assertRaises(serve.BadRequest):
            serve.validate_request({"mode": "dump_restore", "source": SRC})
        with self.assertRaises(serve.BadRequest):
            serve.validate_request({"mode": "dump_restore", "source": SRC, "target": TGT,
                                    "options": {"format": "plain"}})
        req = serve.validate_request({"mode": "dump_restore", "source": SRC, "target": TGT})
        self.assertEqual(req["target"]["database"], "replica")

    def test_rejects_unknown_options_but_ignores_unset_ones(self):
        with self.assertRaises(serve.BadRequest):
            serve.validate_request({"source": SRC, "options": {"schemaonly": True}})
        # Typed clients (the orchestrator) send every key; null/false/[] unknowns are harmless.
        serve.validate_request({"source": SRC, "options": {"future_flag": None, "x": False}})

    def test_conflicts_and_ranges(self):
        bad = [
            {"schema_only": True, "data_only": True},
            {"if_exists": True},
            {"restore_if_exists": True},
            {"format": "tar", "compress": 5},
            {"compress": 10},
            {"compress": True},
            {"restore_jobs": 4, "restore_single_transaction": True},
            {"source_sslmode": "maybe"},
            {"format": "directory"},
        ]
        for opts in bad:
            with self.subTest(opts=opts), self.assertRaises(serve.BadRequest):
                serve.validate_request({"source": SRC, "options": opts})

    def test_connection_validation(self):
        for patch in ({"type": "mysql"}, {"host": ""}, {"host": "h;rm"}, {"port": "99999"},
                      {"database": ""}, {"username": ""}):
            with self.subTest(patch=patch), self.assertRaises(serve.BadRequest):
                serve.validate_request({"source": {**SRC, **patch}})

    def test_pattern_injection_rejected(self):
        for bad in ("--file=/etc/passwd", "-f", "a;b", "a`id`", "a/b", "a\nb", "a'b"):
            with self.subTest(bad=bad), self.assertRaises(serve.BadRequest):
                serve.validate_request({"source": SRC, "include_tables": [bad]})
        req = serve.validate_request({"source": SRC, "include_tables": ['"Sales"."Order Lines"', "public.t_?"]})
        self.assertEqual(len(req["include_tables"]), 2)


KEY_DEST = {"name": "backup", "host": "backup.example.com", "port": "2222", "username": "dumps", "auth": "key",
            "private_key": "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n",
            "passphrase": "kp", "remote_dir": "/srv/dumps/shop"}
PW_DEST = {"host": "db-host", "username": "postgres", "password": "sftp-pw", "remote_dir": "backups"}


class DestinationTest(unittest.TestCase):
    def test_normalises_key_and_password_destinations(self):
        dests = serve.norm_destinations([KEY_DEST, PW_DEST])
        self.assertEqual(dests[0]["port"], 2222)
        self.assertEqual(dests[0]["auth"], "key")
        self.assertEqual(dests[1]["auth"], "password")  # inferred: no private key
        self.assertEqual(dests[1]["name"], "db-host")
        self.assertEqual(dests[1]["port"], 22)
        self.assertEqual(dests[1]["private_key"], "")

    def test_describe_destination_has_no_secrets(self):
        view = json.dumps([serve.describe_destination(d) for d in serve.norm_destinations([KEY_DEST, PW_DEST])])
        for secret in ("sftp-pw", "PRIVATE KEY", '"kp"'):
            self.assertNotIn(secret, view)

    def test_rejects_bad_destinations(self):
        bad = [
            {**PW_DEST, "host": ""},
            {**PW_DEST, "host": "a;b"},
            {**PW_DEST, "port": "70000"},
            {**PW_DEST, "username": ""},
            {**PW_DEST, "remote_dir": ""},
            {**PW_DEST, "remote_dir": "a\nb"},
            {**PW_DEST, "password": ""},
            {**KEY_DEST, "private_key": "not a key"},
            {**PW_DEST, "auth": "kerberos"},
            {**PW_DEST, "host_key_fingerprint": "MD5:aa:bb"},
        ]
        for d in bad:
            with self.subTest(d=d), self.assertRaises(serve.BadRequest):
                serve.norm_destinations([d])
        with self.assertRaises(serve.BadRequest):
            serve.norm_destinations([PW_DEST] * 6)

    def test_fingerprint_matches_openssh_format(self):
        fp = serve.key_fingerprint(b"example-key-blob")
        self.assertTrue(serve.FINGERPRINT.match(fp), fp)
        self.assertFalse(fp.endswith("="))
        ok = serve.norm_destinations([{**PW_DEST, "host_key_fingerprint": fp + "="}])
        self.assertEqual(ok[0]["host_key_fingerprint"], fp, "padding is normalised away")

    def test_upload_hints(self):
        self.assertIn("pinned", serve.upload_hint("BadRequest: host key mismatch for h"))
        self.assertIn("rejected", serve.upload_hint("AuthenticationException: Authentication failed."))
        self.assertIn("cannot write", serve.upload_hint("PermissionError: [Errno 13] Permission denied"))
        self.assertIn("reachable", serve.upload_hint("NoValidConnectionsError: Unable to connect to port 22"))


class BuildTest(unittest.TestCase):
    def test_dump_argv_has_no_secrets_and_strict_names(self):
        req = serve.validate_request({"source": SRC, "include_tables": ["public.orders"], "options": {
            "compress": 6, "no_owner": True, "exclude_table_data": ["audit.*"], "lock_wait_timeout_ms": 5000}})
        argv = serve.build_dump_argv(req, Path("/w/out.dump"))
        self.assertNotIn("ret", " ".join(argv))
        for flag in ("--format=c", "--file=/w/out.dump", "--no-password", "--compress=6", "--no-owner",
                     "--exclude-table-data=audit.*", "--table=public.orders", "--strict-names",
                     "--lock-wait-timeout=5000"):
            self.assertIn(flag, argv)

    def test_no_strict_names_without_includes(self):
        req = serve.validate_request({"source": SRC, "options": {"exclude_schemas": ["tmp"]}})
        self.assertNotIn("--strict-names", serve.build_dump_argv(req, Path("x")))

    def test_restore_argv_quotes_dbname(self):
        req = serve.validate_request({"mode": "dump_restore", "source": SRC,
                                      "target": {**TGT, "database": "it's host=evil"},
                                      "options": {"restore_jobs": 4, "restore_clean": True,
                                                  "restore_if_exists": True}})
        argv = serve.build_restore_argv(req, Path("/w/a.dump"))
        self.assertIn("--dbname=dbname='it\\'s host=evil'", argv)
        self.assertIn("--jobs=4", argv)
        self.assertEqual(argv[-1], "/w/a.dump")

    def test_pgpass_exact_lines_and_escaping(self):
        content = serve.pgpass_content([serve.norm_conn(SRC, "s"), serve.norm_conn(TGT, "t"), None])
        self.assertEqual(content, "src-db:5432:*:reader:s\\:ec\\\\ret\ntgt-db:6543:*:writer:tgt-pw\n")
        self.assertEqual(serve.pgpass_content([serve.norm_conn({**SRC, "password": ""}, "s")]), "")

    def test_env_drops_inherited_pg_vars(self):
        env = serve.libpq_env({"PATH": "/bin", "PGPASSWORD": "leak", "PGHOST": "old"},
                              serve.norm_conn(SRC, "s"), Path("/w/.pgpass"), "require")
        self.assertNotIn("PGPASSWORD", env)
        self.assertEqual(env["PGHOST"], "src-db")
        self.assertEqual(env["PGPASSFILE"], "/w/.pgpass")
        self.assertEqual(env["PGSSLMODE"], "require")
        self.assertEqual(env["PATH"], "/bin")

    def test_toc_schemas(self):
        toc = """;
; Archive created at 2026-11-02
;
216; 1259 16385 TABLE shop orders postgres
3452; 0 16385 TABLE DATA shop orders postgres
217; 1259 16390 SEQUENCE shop orders_id_seq postgres
230; 1259 16400 TABLE Sales Order postgres
231; 1259 16401 TABLE public t postgres
"""
        schemas, skipped = serve.toc_schemas(toc)
        self.assertEqual(schemas, ["shop", "Sales", "public"])
        self.assertEqual(skipped, [])
        self.assertEqual(serve.create_schemas_sql(["shop"]), 'CREATE SCHEMA IF NOT EXISTS "shop";')
        self.assertEqual(serve.toc_schemas("9; 1 2 TABLE we-ird t o\n")[1], ["we-ird"])

    def test_hints(self):
        self.assertIn("schema is missing", serve.hint_for('ERROR:  schema "shop" does not exist'))
        self.assertIn("older", serve.hint_for("pg_dump: error: aborting because of server version mismatch"))
        self.assertIn("Authentication", serve.hint_for('FATAL:  password authentication failed for user "x"'))
        self.assertIsNone(serve.hint_for("all good"))


FAKE_DUMP = """#!/bin/sh
for a in "$@"; do case "$a" in --file=*) f="${a#--file=}";; esac; done
echo 'pg_dump: dumping contents of table "public.a"' >&2
echo 'pg_dump: dumping contents of table "public.b"' >&2
{ echo "host=$PGHOST port=$PGPORT user=$PGUSER db=$PGDATABASE leak=${PGPASSWORD:-none}"; cat "$PGPASSFILE"; } > "$f"
"""
FAKE_RESTORE_OK = """#!/bin/sh
echo "pg_restore: processing data for table \\"public.a\\"" >&2
echo "restore host=$PGHOST user=$PGUSER args=$*"
"""
FAKE_RESTORE_IGNORED = """#!/bin/sh
echo 'pg_restore: error: relation "a" already exists' >&2
echo 'pg_restore: warning: errors ignored on restore: 3' >&2
exit 1
"""
FAKE_FAIL = """#!/bin/sh
echo 'pg_dump: error: connection to server at "src-db" failed: FATAL:  password authentication failed for user "reader"' >&2
exit 1
"""
FAKE_SLOW = "#!/bin/sh\nexec sleep 30\n"


class SidecarCase(unittest.TestCase):
    """Temp work root + a bin/ dir for fake client binaries; no tests of its own."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.saved = {k: getattr(serve, k) for k in ("WORK_ROOT", "JOBS_DIR", "PG_DUMP", "PG_RESTORE",
                                                       "MIN_FREE_MB", "MAX_CONCURRENT")}
        serve.WORK_ROOT = root
        serve.JOBS_DIR = root / "jobs"
        serve.JOBS_DIR.mkdir()
        serve.MIN_FREE_MB = 0
        serve.MAX_CONCURRENT = 2
        self.bin = root / "bin"
        self.bin.mkdir()

    def tearDown(self):
        for k, v in self.saved.items():
            setattr(serve, k, v)
        self.tmp.cleanup()

    def script(self, name: str, body: str) -> str:
        p = self.bin / name
        p.write_text(body)
        p.chmod(0o700)
        return str(p)


class JobTest(SidecarCase):
    def wait(self, job_id: str, timeout: float = 15) -> dict:
        end = time.time() + timeout
        while time.time() < end:
            meta = serve.job_status(job_id)
            if meta["status"] not in serve.ACTIVE:
                return meta
            time.sleep(0.05)
        self.fail(f"job {job_id} still {serve.job_status(job_id)['status']}")

    def test_dump_mode_end_to_end(self):
        serve.PG_DUMP = self.script("pg_dump", FAKE_DUMP)
        job = serve.start_job({"source": SRC, "name": "shop nightly", "profile_id": "p1"})
        meta = self.wait(job["id"])
        self.assertEqual(meta["status"], "succeeded", meta)
        self.assertEqual(meta["tables_dumped"], 2)
        self.assertTrue(meta["artifact"]["name"].startswith("shop-nightly-"))
        self.assertNotIn("path", meta["artifact"])
        self.assertEqual(len(meta["artifact"]["sha256"]), 64)
        self.assertEqual(meta["download_path"], f"/v1/jobs/{job['id']}/file")

        work = serve.job_dir(job["id"])
        dump = (work / meta["artifact"]["name"]).read_text()
        self.assertIn("host=src-db port=5432 user=reader db=shop leak=none", dump)
        self.assertIn("src-db:5432:*:reader:s\\:ec\\\\ret", dump)
        self.assertFalse((work / ".pgpass").exists(), "passfile must be removed after the job")
        on_disk = (work / "job.json").read_text() + (work / "pgdump.log").read_text()
        self.assertNotIn("s:ec", on_disk.replace("s\\\\:ec", ""), "no plaintext secret in job.json/log")
        self.assertNotIn("tgt-pw", on_disk)

        self.assertTrue(serve.delete_job(job["id"]))
        self.assertFalse(work.exists())

    def test_passfile_is_0600_while_running(self):
        serve.PG_DUMP = self.script("pg_dump", FAKE_SLOW)
        job = serve.start_job({"source": SRC})
        pf = serve.job_dir(job["id"]) / ".pgpass"
        self.assertEqual(stat.S_IMODE(pf.stat().st_mode), 0o600)
        serve.stop_job(job["id"])
        meta = self.wait(job["id"])
        self.assertEqual(meta["status"], "stopped")
        self.assertFalse(pf.exists())

    def test_dump_restore_success_and_ignored_errors(self):
        serve.PG_DUMP = self.script("pg_dump", FAKE_DUMP)
        serve.PG_RESTORE = self.script("pg_restore", FAKE_RESTORE_OK)
        job = serve.start_job({"mode": "dump_restore", "source": SRC, "target": TGT})
        meta = self.wait(job["id"])
        self.assertEqual(meta["status"], "succeeded", meta)
        self.assertEqual(meta["tables_restored"], 1)
        log = serve.tail_log(serve.job_dir(job["id"]) / "pgdump.log")
        self.assertIn("restore host=tgt-db user=writer", log)

        serve.PG_RESTORE = self.script("pg_restore", FAKE_RESTORE_IGNORED)
        job = serve.start_job({"mode": "dump_restore", "source": SRC, "target": TGT})
        meta = self.wait(job["id"])
        self.assertEqual(meta["status"], "completed_with_errors", meta)
        self.assertEqual(meta["errors_ignored"], 3)
        self.assertIn("already exist", meta["hint"])
        self.assertIn("download_path", meta, "archive is kept when the restore fails")

    def test_failure_has_hint_and_drops_partial_file(self):
        serve.PG_DUMP = self.script("pg_dump", FAKE_FAIL)
        job = serve.start_job({"source": SRC})
        meta = self.wait(job["id"])
        self.assertEqual(meta["status"], "failed")
        self.assertEqual(meta["exit_code"], 1)
        self.assertIn("Authentication", meta["hint"])
        self.assertNotIn("download_path", meta)

    def test_concurrency_and_per_profile_guard(self):
        serve.PG_DUMP = self.script("pg_dump", FAKE_SLOW)
        a = serve.start_job({"source": SRC, "profile_id": "p1"})
        with self.assertRaises(serve.BadRequest) as dup:
            serve.start_job({"source": SRC, "profile_id": "p1"})
        self.assertEqual(dup.exception.code, 409)
        b = serve.start_job({"source": SRC, "profile_id": "p2"})
        with self.assertRaises(serve.BadRequest) as full:
            serve.start_job({"source": SRC, "profile_id": "p3"})
        self.assertEqual(full.exception.code, 429)
        with self.assertRaises(serve.BadRequest) as busy:
            serve.delete_job(a["id"])
        self.assertEqual(busy.exception.code, 409)
        for j in (a, b):
            serve.stop_job(j["id"])
            self.wait(j["id"])

    def fake_uploader(self, fail_hosts=(), seen=None):
        def upload(dest, local, digest, progress, stopped):
            if seen is not None:
                seen.append((dest["host"], dest["password"] or dest["private_key"][:20], local.read_bytes()[:4]))
            progress(local.stat().st_size, local.stat().st_size)
            if dest["host"] in fail_hosts:
                raise PermissionError(13, "Permission denied")
            return {"remote_path": f"{dest['remote_dir']}/{local.name}", "bytes": local.stat().st_size,
                    "host_key_fingerprint": "SHA256:" + "A" * 43, "host_key_verified": False}
        return upload

    def test_upload_all_destinations_then_remove_local_copy(self):
        serve.PG_DUMP = self.script("pg_dump", FAKE_DUMP)
        seen = []
        saved = serve.UPLOADER
        serve.UPLOADER = self.fake_uploader(seen=seen)
        try:
            job = serve.start_job({"source": SRC, "destinations": [KEY_DEST, PW_DEST]})
            meta = self.wait(job["id"])
        finally:
            serve.UPLOADER = saved
        self.assertEqual(meta["status"], "succeeded", meta)
        self.assertEqual([u["status"] for u in meta["uploads"]], ["uploaded", "uploaded"])
        self.assertEqual(meta["uploads"][0]["remote_path"], f"/srv/dumps/shop/{meta['artifact']['name']}")
        self.assertEqual([h for h, _, _ in seen], ["backup.example.com", "db-host"])
        self.assertTrue(meta["artifact"]["local_deleted"])
        self.assertNotIn("download_path", meta, "local copy removed after verified uploads")
        on_disk = (serve.job_dir(job["id"]) / "job.json").read_text() + \
            (serve.job_dir(job["id"]) / "pgdump.log").read_text()
        for secret in ("sftp-pw", "PRIVATE KEY", '"kp"'):
            self.assertNotIn(secret, on_disk)

    def test_failed_upload_keeps_local_file_and_reports(self):
        serve.PG_DUMP = self.script("pg_dump", FAKE_DUMP)
        saved = serve.UPLOADER
        serve.UPLOADER = self.fake_uploader(fail_hosts=("db-host",))
        try:
            job = serve.start_job({"source": SRC, "destinations": [KEY_DEST, PW_DEST]})
            meta = self.wait(job["id"])
        finally:
            serve.UPLOADER = saved
        self.assertEqual(meta["status"], "completed_with_errors", meta)
        self.assertIn("db-host", meta["error"])
        self.assertEqual([u["status"] for u in meta["uploads"]], ["uploaded", "failed"])
        self.assertIn("cannot write", meta["uploads"][1]["hint"])
        self.assertNotIn("hint", meta, "database hints must not be shown for SFTP failures")
        self.assertIn("download_path", meta, "local copy kept when any upload fails")

    def test_keep_local(self):
        serve.PG_DUMP = self.script("pg_dump", FAKE_DUMP)
        saved = serve.UPLOADER
        serve.UPLOADER = self.fake_uploader()
        try:
            job = serve.start_job({"source": SRC, "destinations": [PW_DEST], "options": {"keep_local": True}})
            meta = self.wait(job["id"])
        finally:
            serve.UPLOADER = saved
        self.assertEqual(meta["status"], "succeeded")
        self.assertIn("download_path", meta)

    def test_stop_during_upload(self):
        serve.PG_DUMP = self.script("pg_dump", FAKE_DUMP)
        saved = serve.UPLOADER
        gate = __import__("threading").Event()

        def slow(dest, local, digest, progress, stopped):
            gate.set()
            while not stopped():
                time.sleep(0.02)
            raise serve.UploadStopped()

        serve.UPLOADER = slow
        try:
            job = serve.start_job({"source": SRC, "destinations": [PW_DEST, KEY_DEST]})
            self.assertTrue(gate.wait(10))
            serve.stop_job(job["id"])
            meta = self.wait(job["id"])
        finally:
            serve.UPLOADER = saved
        self.assertEqual(meta["status"], "stopped")
        self.assertEqual([u["status"] for u in meta["uploads"]], ["stopped", "pending"])

    def wait_send(self, job_id: str, timeout: float = 10) -> dict:
        end = time.time() + timeout
        while time.time() < end:
            meta = serve.job_status(job_id)
            if not meta.get("sending"):
                return meta
            time.sleep(0.05)
        self.fail(f"send for {job_id} still running")

    def test_adhoc_send_appends_upload_and_keeps_local_file(self):
        serve.PG_DUMP = self.script("pg_dump", FAKE_DUMP)
        job = serve.start_job({"source": SRC})
        self.assertEqual(self.wait(job["id"])["status"], "succeeded")
        seen = []
        saved = serve.UPLOADER
        serve.UPLOADER = self.fake_uploader(seen=seen)
        try:
            out = serve.send_file(job["id"], {"destination": PW_DEST})
            self.assertTrue(out["sending"])
            meta = self.wait_send(job["id"])
        finally:
            serve.UPLOADER = saved
        self.assertEqual(meta["status"], "succeeded", "a send never changes the run result")
        self.assertEqual(len(meta["uploads"]), 1)
        up = meta["uploads"][0]
        self.assertEqual((up["status"], up["adhoc"], up["host"]), ("uploaded", True, "db-host"))
        self.assertIn("download_path", meta, "a send never removes the local file")
        self.assertEqual([h for h, _, _ in seen], ["db-host"])
        on_disk = (serve.job_dir(job["id"]) / "job.json").read_text()
        self.assertNotIn("sftp-pw", on_disk)
        self.assertIn("send (ad hoc)", serve.tail_log(serve.job_dir(job["id"]) / "pgdump.log"))

    def test_adhoc_send_failure_has_hint_and_guards(self):
        serve.PG_DUMP = self.script("pg_dump", FAKE_DUMP)
        job = serve.start_job({"source": SRC})
        self.wait(job["id"])
        with self.assertRaises(serve.BadRequest):
            serve.send_file(job["id"], {})
        with self.assertRaises(serve.BadRequest):
            serve.send_file(job["id"], {"destination": {"host": "x"}})
        with self.assertRaises(serve.BadRequest) as missing:
            serve.send_file("no-such-job", {"destination": PW_DEST})
        self.assertEqual(missing.exception.code, 404)

        gate = __import__("threading").Event()

        def slow(dest, local, digest, progress, stopped):
            gate.set()
            while not stopped():
                time.sleep(0.02)
            raise serve.UploadStopped()

        saved = serve.UPLOADER
        serve.UPLOADER = slow
        try:
            serve.send_file(job["id"], {"destination": PW_DEST})
            self.assertTrue(gate.wait(5))
            with self.assertRaises(serve.BadRequest) as dup:
                serve.send_file(job["id"], {"destination": KEY_DEST})
            self.assertEqual(dup.exception.code, 409)
            with self.assertRaises(serve.BadRequest) as busy:
                serve.delete_job(job["id"])
            self.assertEqual(busy.exception.code, 409)
            serve.stop_job(job["id"])
            meta = self.wait_send(job["id"])
            self.assertEqual(meta["uploads"][-1]["status"], "stopped")

            serve.UPLOADER = self.fake_uploader(fail_hosts=("db-host",))
            serve.send_file(job["id"], {"destination": PW_DEST})
            meta = self.wait_send(job["id"])
        finally:
            serve.UPLOADER = saved
        self.assertEqual([u["status"] for u in meta["uploads"]], ["stopped", "failed"])
        self.assertIn("cannot write", meta["uploads"][-1]["hint"])

        (serve.job_dir(job["id"]) / serve.read_json(serve.job_dir(job["id"]) / "job.json")["artifact"]["name"]).unlink()
        with self.assertRaises(serve.BadRequest) as gone:
            serve.send_file(job["id"], {"destination": PW_DEST})
        self.assertEqual(gone.exception.code, 410)

    def test_recover_on_start_marks_orphans_and_removes_passfiles(self):
        work = serve.JOBS_DIR / "orphan-1"
        work.mkdir()
        (work / ".pgpass").write_text("x")
        (work / "job.json").write_text(json.dumps({"status": "running"}))
        serve.recover_on_start()
        self.assertFalse((work / ".pgpass").exists())
        self.assertEqual(serve.read_json(work / "job.json")["status"], "failed")

    def test_job_id_path_traversal_rejected(self):
        with self.assertRaises(serve.BadRequest):
            serve.job_dir("..")



FAKE_PSQL_OK = """#!/bin/sh
# Record what libpq would see (env + passfile), then answer like the real query.
{ echo "host=$PGHOST port=$PGPORT user=$PGUSER db=$PGDATABASE ssl=${PGSSLMODE:-none} leak=${PGPASSWORD:-none}"; cat "$PGPASSFILE"; } > "$(dirname "$0")/seen"
ls -a "$(dirname "$PGPASSFILE")" > "$(dirname "$0")/tmpdir"
echo '{"server_version": "%s", "server_version_num": %s, "current_user": "reader", "database_bytes": 1048576, "ssl": false, "schemas": [{"name": "public", "tables": 2, "bytes": 8192}, '
echo ' {"name": "sales", "tables": 0, "bytes": 0}]}'
"""
FAKE_PSQL_AUTH = """#!/bin/sh
echo 'psql: error: connection to server at "src-db" (10.0.0.5), port 5432 failed: FATAL:  password authentication failed for user "reader"' >&2
exit 2
"""
FAKE_PG_DUMP_VERSION = "#!/bin/sh\necho 'pg_dump (PostgreSQL) 18.3 (Debian 18.3-1.pgdg13+1)'\n"


class TestConnectionTest(SidecarCase):
    def setUp(self):
        super().setUp()
        self.saved.update({k: getattr(serve, k) for k in ("PSQL",)})
        serve.PG_DUMP = self.script("pg_dump", FAKE_PG_DUMP_VERSION)

    def test_ok_describes_database_without_secrets(self):
        serve.PSQL = self.script("psql", FAKE_PSQL_OK % ("17.7", 170007))
        res = serve.test_connection({"connection": SRC, "sslmode": "require"})
        self.assertTrue(res["ok"], res)
        self.assertEqual(res["server_major"], 17)
        self.assertEqual(res["client_major"], 18)
        self.assertTrue(res["compatible"])
        self.assertEqual(res["schemas"], [{"name": "public", "tables": 2, "bytes": 8192},
                                          {"name": "sales", "tables": 0, "bytes": 0}])
        self.assertEqual(res["target"], "reader@src-db:5432/shop")
        self.assertNotIn("server_version_num", res)
        self.assertNotIn("s:ec", json.dumps(res))
        seen = (self.bin / "seen").read_text()
        self.assertIn("host=src-db port=5432 user=reader db=shop ssl=require leak=none", seen)
        self.assertIn("src-db:5432:*:reader:s\\:ec\\\\ret", seen)
        self.assertEqual([p for p in serve.WORK_ROOT.iterdir() if p.name.startswith(".test-")], [],
                         "temporary passfile directory must be removed")

    def test_newer_server_than_client_is_flagged(self):
        serve.PSQL = self.script("psql", FAKE_PSQL_OK % ("19.0", 190000))
        res = serve.test_connection({"connection": SRC})
        self.assertTrue(res["ok"])
        self.assertFalse(res["compatible"])
        self.assertIn("PG_MAJOR=19", res["hint"])

    def test_failure_is_an_answer_with_hint(self):
        serve.PSQL = self.script("psql", FAKE_PSQL_AUTH)
        res = serve.test_connection({"connection": SRC})
        self.assertFalse(res["ok"])
        self.assertIn("password authentication failed", res["error"])
        self.assertIn("username/password", res["hint"])

    def test_rejects_bad_input(self):
        with self.assertRaises(serve.BadRequest):
            serve.test_connection({"connection": {**SRC, "host": "a;b"}})
        with self.assertRaises(serve.BadRequest):
            serve.test_connection({"connection": SRC, "sslmode": "maybe"})

    def test_major_of(self):
        self.assertEqual(serve.major_of("pg_dump (PostgreSQL) 18.3 (Debian 18.3-1.pgdg13+1)"), 18)
        self.assertEqual(serve.major_of("pg_dump (PostgreSQL) 9.6.24"), 9)
        self.assertIsNone(serve.major_of("unavailable"))


if __name__ == "__main__":
    unittest.main()
