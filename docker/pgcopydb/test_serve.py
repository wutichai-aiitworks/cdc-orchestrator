"""Unit tests for the cdc-pgcopydb sidecar.  Run: cd docker/pgcopydb && python3 -m unittest -v"""

from __future__ import annotations

import json
import stat
import tempfile
import time
import unittest
from pathlib import Path

import serve
from jobstore import JobStore

SRC = {"host": "src-db", "port": "5432", "database": "shop", "username": "reader", "password": "s:ec\\ret"}
TGT = {"host": "tgt-db", "port": "6543", "database": "copy", "username": "writer", "password": "tgt-pw"}

# Records what libpq would see, then succeeds (or fails with FAIL=1 in its argv).
FAKE = """#!/bin/sh
work=$(dirname "$(dirname "$0")")
{ echo "argv=$*"; echo "passfile=$PGPASSFILE"; echo "leak=${PGPASSWORD:-none}"; ls -l "$PGPASSFILE"; cat "$PGPASSFILE"; } > "$SEEN"
echo "STEP 1: fetch source database tables"
echo "STEP 4: starting 2 table-data COPY processes"
"""


class FiltersTest(unittest.TestCase):
    def test_include_tables_use_a_section_pgcopydb_applies(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = serve.write_filters(Path(tmp), ["public.orders", " sales.lines ", ""])
            self.assertEqual(path.read_text(), "[include-only-table]\npublic.orders\nsales.lines\n")

    def test_no_filter_file_without_tables(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(serve.write_filters(Path(tmp), []))


class RunTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.saved = (serve.STORE, serve.JOBS_DIR, serve.PGCOPYDB)
        serve.JOBS_DIR = root / "jobs"
        serve.STORE = JobStore(serve.JOBS_DIR, "clone.log")
        (root / "bin").mkdir()
        fake = root / "bin" / "pgcopydb"
        fake.write_text(FAKE)
        fake.chmod(0o700)
        serve.PGCOPYDB = str(fake)
        self.seen = root / "seen.txt"
        import os
        os.environ["SEEN"] = str(self.seen)

    def tearDown(self):
        serve.STORE, serve.JOBS_DIR, serve.PGCOPYDB = self.saved
        self.tmp.cleanup()

    def wait(self, job_id):
        for _ in range(200):
            meta = serve.STORE.meta(job_id)
            if meta["status"] not in ("starting", "running"):
                return meta
            time.sleep(0.05)
        self.fail("still running")

    def test_passwords_never_reach_argv_job_or_log(self):
        job = serve.start_job({"profile_id": "p1", "name": "seed", "source": SRC, "target": TGT, "include_tables": ["public.orders"], "options": {"table_jobs": 2}})
        meta = self.wait(job["id"])
        self.assertEqual(meta["status"], "succeeded", meta)
        seen = self.seen.read_text()
        self.assertIn("--source postgresql://reader@src-db:5432/shop --target postgresql://writer@tgt-db:6543/copy", seen)
        self.assertIn("leak=none", seen)
        self.assertIn("-rw-------", seen, "passfile is 0600")
        self.assertIn("src-db:5432:*:reader:s\\:ec\\\\ret", seen)
        self.assertIn("tgt-db:6543:*:writer:tgt-pw", seen)
        work = serve.JOBS_DIR / job["id"]
        self.assertFalse((work / ".pgpass").exists(), "passfile removed when the run ends")
        on_disk = (work / "job.json").read_text() + serve.STORE.tail_log(job["id"])
        for secret in ("s:ec", "tgt-pw"):
            self.assertNotIn(secret, on_disk)
        self.assertEqual(meta["profile_id"], "p1")

    def test_each_run_is_fresh_unless_resuming(self):
        first = serve.start_job({"profile_id": "p1", "source": SRC, "target": TGT})
        self.wait(first["id"])
        state = serve.JOBS_DIR / first["id"] / "clone"
        state.mkdir()
        (state / "snapshot").write_text("00000003-0000000B-1")
        fresh = serve.start_job({"profile_id": "p1", "source": SRC, "target": TGT})
        self.wait(fresh["id"])
        self.assertFalse((serve.JOBS_DIR / fresh["id"] / "clone" / "snapshot").exists())
        resumed = serve.start_job({"profile_id": "p1", "source": SRC, "target": TGT, "options": {"resume": True}})
        meta = self.wait(resumed["id"])
        self.assertEqual(meta["resumed_from"], first["id"])
        self.assertTrue((serve.JOBS_DIR / resumed["id"] / "clone" / "snapshot").exists(), "resume continues the previous state")


if __name__ == "__main__":
    unittest.main()
