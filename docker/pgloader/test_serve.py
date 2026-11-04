"""Unit tests for the cdc-pgloader sidecar.  Run: cd docker/pgloader && python3 -m unittest -v"""

from __future__ import annotations

import sqlite3
import stat
import tempfile
import unittest
from pathlib import Path

import serve

PG = {"type": "postgres", "host": "pg", "port": "5432", "database": "lake", "username": "loader", "password": "pg-pw"}


class LoadFileTest(unittest.TestCase):
    def test_load_file_is_private(self):
        with tempfile.TemporaryDirectory() as tmp:
            body = {"source": {"type": "mysql", "host": "my", "database": "shop", "username": "u", "password": "my-pw"}, "target": PG, "options": {}}
            path = serve.write_load_file(Path(tmp), body)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            self.assertIn("mysql://u:my-pw@my:3306/shop", path.read_text())
            serve.cleanup_secrets(Path(tmp))
            self.assertFalse(path.exists())


class FileProbeTest(unittest.TestCase):
    def test_csv(self):
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / "orders.csv"
            f.write_text("id;name;amount\n1;a;2\n")
            ok = serve.test_file({"database": str(f)}, "csv", {"csv_delimiter": ";"})
            self.assertTrue(ok["ok"])
            self.assertIn("3 columns", ok["facts"])
            wrong = serve.test_file({"database": str(f)}, "csv", {"csv_delimiter": ","})
            self.assertIn("different separator", wrong["warning"])
            missing = serve.test_file({"database": str(Path(tmp) / "nope.csv")}, "csv", {})
            self.assertFalse(missing["ok"])
            self.assertIn("mounted into cdc-pgloader", missing["hint"])

    def test_sqlite(self):
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / "shop.sqlite"
            db = sqlite3.connect(f)
            db.execute("create table t(id int)")
            db.close()
            self.assertIn("1 table", serve.test_file({"database": str(f)}, "sqlite", {})["facts"])
            junk = Path(tmp) / "junk.sqlite"
            junk.write_text("not a database")
            self.assertFalse(serve.test_file({"database": str(junk)}, "sqlite", {})["ok"])


class DryRunTest(unittest.TestCase):
    """pgloader --dry-run prints one Success/Failed line per connection and exits 0 either way."""

    def run_fake(self, output: str) -> dict:
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / "pgloader"
            fake.write_text("#!/bin/sh\ncat <<'OUT'\n" + output + "\nOUT\n")
            fake.chmod(0o700)
            saved = (serve.PGLOADER, serve.WORK_ROOT)
            serve.PGLOADER, serve.WORK_ROOT = str(fake), Path(tmp)
            try:
                return serve.test_database_source({"type": "mysql", "host": "my", "database": "shop", "username": "root", "password": "x"}, "mysql")
            finally:
                serve.PGLOADER, serve.WORK_ROOT = saved

    def test_success_ignores_the_dummy_target(self):
        res = self.run_fake(
            "LOG Success, opened #<MYSQL-CONNECTION mysql://root@my:3306/shop {1}>.\n"
            "FATAL Failed to connect to #<PGSQL-CONNECTION pgsql://probe@127.0.0.1:1/probe {2}>: refused"
        )
        self.assertTrue(res["ok"], res)

    def test_failure_reason_and_hint(self):
        res = self.run_fake(
            "FATAL Failed to connect to #<MYSQL-CONNECTION mysql://root@my:3306/shop {1}>: Failed to connect to mysql at \"my\" (port 3306) as user \"root\": MySQL Error [1045]: \"Access denied for user 'root'\""
        )
        self.assertFalse(res["ok"])
        self.assertIn("Access denied", res["error"])
        self.assertIn("rejected the login", res["hint"])


if __name__ == "__main__":
    unittest.main()
