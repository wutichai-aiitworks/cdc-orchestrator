"""Unit tests for the shared sidecar job store, HTTP contract and probes.
Run: cd docker/sidecar_common && python3 -m unittest -v"""

from __future__ import annotations

import json
import os
import stat
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path

import http_api
import jobstore
import probes


def wait_done(store: jobstore.JobStore, job_id: str, timeout: float = 10) -> dict:
    end = time.time() + timeout
    while time.time() < end:
        meta = store.meta(job_id)
        if meta["status"] not in jobstore.ACTIVE:
            return meta
        time.sleep(0.05)
    raise AssertionError(f"{job_id} still {store.meta(job_id)['status']}")


class JobStoreTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.store = jobstore.JobStore(Path(self.tmp.name) / "jobs", "run.log", retain_per_profile=2)

    def tearDown(self):
        self.tmp.cleanup()

    def start(self, profile: str, argv: list[str], **kw) -> dict:
        job_id, work, _ = self.store.new_job({"profile_id": profile, "name": "t"})
        return self.store.spawn(job_id, argv, **kw)

    def test_each_run_is_a_new_job_of_its_transfer(self):
        a = self.start("p1", ["sh", "-c", "echo one"])
        wait_done(self.store, a["id"])
        b = self.start("p1", ["sh", "-c", "echo two; exit 3"])
        meta = wait_done(self.store, b["id"])
        self.assertNotEqual(a["id"], b["id"])
        self.assertEqual(meta["status"], "failed")
        self.assertEqual(meta["exit_code"], 3)
        self.assertEqual([m["id"] for m in self.store.list("p1")], [b["id"], a["id"]])
        self.assertIn("two", self.store.tail_log(b["id"]))
        self.assertEqual(self.store.list("other"), [])

    def test_one_active_run_per_transfer(self):
        a = self.start("p1", ["sleep", "5"])
        with self.assertRaises(jobstore.BusyError):
            self.store.new_job({"profile_id": "p1"})
        b = self.start("p2", ["sh", "-c", "true"])  # another transfer may run
        wait_done(self.store, b["id"])
        meta = self.store.stop(a["id"], grace_seconds=2)
        self.assertEqual(meta["status"], "stopped")

    def test_on_exit_runs_when_the_process_ends(self):
        secret = Path(self.tmp.name) / "secret"
        jobstore.write_secret(secret, "pw")
        self.assertEqual(stat.S_IMODE(secret.stat().st_mode), 0o600)
        job = self.start("p1", ["sh", "-c", "true"], on_exit=lambda jid, code: secret.unlink())
        wait_done(self.store, job["id"])
        time.sleep(0.1)
        self.assertFalse(secret.exists(), "secrets are removed as soon as the run ends")

    def test_retention_keeps_newest_finished_runs(self):
        ids = []
        for i in range(4):
            job = self.start("p1", ["sh", "-c", "true"])
            wait_done(self.store, job["id"])
            ids.append(job["id"])
            time.sleep(0.01)
        self.start("p1", ["sh", "-c", "true"])  # pruning happens when a run starts
        kept = {m["id"] for m in self.store.list("p1")}
        self.assertNotIn(ids[0], kept)
        self.assertIn(ids[3], kept)

    def test_legacy_job_without_profile_id_belongs_to_its_id(self):
        legacy = self.store.jobs_dir / "0b2c0f3e-0000-0000-0000-000000000001"
        jobstore.write_json(legacy / "job.json", {"status": "succeeded", "created_at": "2026-01-01T00:00:00+00:00"})
        self.assertEqual(self.store.list(legacy.name)[0]["profile_id"], legacy.name)
        # Older orchestrators send the profile id as "id": it becomes the profile_id, not the job id.
        job_id, _, meta = self.store.new_job({"id": "prof-1"})
        self.assertEqual(meta["profile_id"], "prof-1")
        self.assertNotEqual(job_id, "prof-1")

    def test_recover_on_start_fails_orphans_and_cleans(self):
        job_id, work, _ = self.store.new_job({"profile_id": "p1"})
        jobstore.write_secret(work / ".pgpass", "x")
        self.store.update(job_id, status="running")
        self.store.recover_on_start(lambda w: (w / ".pgpass").unlink(missing_ok=True))
        self.assertEqual(self.store.meta(job_id)["status"], "failed")
        self.assertFalse((work / ".pgpass").exists())

    def test_bad_ids_are_not_found(self):
        for bad in ("..", "../etc", "a/b", ""):
            with self.assertRaises(jobstore.NotFound):
                self.store.meta(bad)


class ProbeTest(unittest.TestCase):
    def test_pgpass_lines_are_exact_and_escaped(self):
        text = probes.pgpass_content([
            {"host": "src", "port": "5432", "username": "u", "password": "p:a\\ss"},
            {"host": "tgt", "port": "", "username": "w", "password": ""},
        ])
        self.assertEqual(text, "src:5432:*:u:p\\:a\\\\ss\n")

    def test_env_drops_inherited_pg_variables(self):
        env = probes.pg_env({"PGPASSWORD": "leak", "PATH": "/bin"}, Path("/tmp/x"))
        self.assertNotIn("PGPASSWORD", env)
        self.assertEqual(env["PGPASSFILE"], "/tmp/x")

    def test_hints(self):
        self.assertIn("rejected the login", probes.hint_for('FATAL: password authentication failed for user "u"'))
        self.assertIn("does not resolve", probes.hint_for("could not translate host name \"x\""))
        self.assertIn("did not accept", probes.hint_for("connect: connection refused"))

    def test_norm_conn_validates(self):
        with self.assertRaises(probes.BadRequest):
            probes.norm_conn({"host": "a;b", "database": "d", "username": "u"})
        c = probes.norm_conn({"host": "h", "username": ""}, default_port="6379", need_database=False, need_user=False)
        self.assertEqual(c["port"], "6379")

    def test_pg_test_failure_is_an_answer(self):
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / "psql"
            fake.write_text("#!/bin/sh\necho 'psql: error: FATAL:  password authentication failed for user \"u\"' >&2\nexit 2\n")
            fake.chmod(0o700)
            res = probes.pg_test({"host": "h", "database": "d", "username": "u", "password": "s3cret"}, tmp_root=Path(tmp), psql=str(fake), dump_client=None)
            self.assertFalse(res["ok"])
            self.assertIn("rejected the login", res["hint"])
            self.assertNotIn("s3cret", json.dumps(res))
            self.assertEqual([p.name for p in Path(tmp).iterdir() if p.name.startswith(".test-")], [], "temp passfile dir removed")


class HttpTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        store = jobstore.JobStore(Path(self.tmp.name) / "jobs", "run.log")

        def start(body):
            job_id, _, _ = store.new_job(body)
            return store.spawn(job_id, ["sh", "-c", "echo hello"])

        tool = http_api.Tool(tool_id="demo", service="cdc-demo", store=store, capability=lambda: {"x": 1}, start=start,
                             test=lambda body: {"ok": True, "target": "h", "facts": ["fine"]}, api_token="tok")
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), http_api.make_handler(tool))
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}"

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.tmp.cleanup()

    def call(self, method, path, body=None, token="tok"):
        req = urllib.request.Request(self.base + path, method=method, data=json.dumps(body).encode() if body is not None else None,
                                     headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=5) as r:
                return r.status, json.loads(r.read())
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read())

    def test_contract(self):
        self.assertEqual(self.call("GET", "/health", token="wrong")[0], 401)
        self.assertEqual(self.call("GET", "/v1/discover")[1]["tool_id"], "demo")
        self.assertEqual(self.call("POST", "/v1/jobs", {"source": {}})[0], 400)
        code, job = self.call("POST", "/v1/jobs", {"source": {"h": 1}, "target": {"h": 2}, "profile_id": "p1"})
        self.assertEqual(code, 202)
        for _ in range(100):
            if self.call("GET", f"/v1/jobs/{job['id']}")[1]["status"] == "succeeded":
                break
            time.sleep(0.05)
        self.assertEqual(self.call("GET", "/v1/jobs?profile_id=p1")[1]["jobs"][0]["id"], job["id"])
        self.assertEqual(self.call("GET", "/v1/jobs?profile_id=nope")[1]["jobs"], [])
        self.assertIn("hello", self.call("GET", f"/v1/jobs/{job['id']}/logs")[1]["log"])
        self.assertEqual(self.call("GET", "/v1/jobs/missing-job")[0], 404)
        self.assertEqual(self.call("POST", "/v1/test-connection", {"connection": {}})[1]["facts"], ["fine"])


if __name__ == "__main__":
    unittest.main()
