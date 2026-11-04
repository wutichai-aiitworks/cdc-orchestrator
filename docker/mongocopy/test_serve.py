"""Unit tests for the cdc-mongocopy sidecar.  Run: cd docker/mongocopy && python3 -m unittest -v"""

from __future__ import annotations

import shlex
import stat
import tempfile
import unittest
from pathlib import Path

import serve

SRC = {"host": "src", "port": "27017", "database": "app", "username": "root", "password": "p@ss"}
TGT = {"host": "tgt", "port": "27017", "database": "app_copy", "username": "root", "password": "p@ss"}


def commands(body: dict) -> tuple[list[str], list[str], dict[str, str], str]:
    with tempfile.TemporaryDirectory() as tmp:
        script = serve.write_run_script(Path(tmp), body).read_text()
        configs = {p.name: p.read_text() for p in Path(tmp).glob("*.yaml")}
        modes = {p.name: stat.S_IMODE(p.stat().st_mode) for p in Path(tmp).glob("*.yaml")}
        assert set(modes.values()) == {0o600}, modes
    dump, restore = (shlex.split(line) for line in script.splitlines() if line.startswith("'mongo"))
    return dump, restore, configs, script


class RestoreTest(unittest.TestCase):
    def test_passwords_only_in_0600_config_files(self):
        dump, restore, configs, script = commands({"source": SRC, "target": TGT, "options": {}})
        self.assertNotIn("p@ss", script)
        self.assertNotIn("p%40ss", script)
        self.assertTrue(any(a.startswith("--config=") for a in dump))
        self.assertIn('uri: "mongodb://root:p%40ss@src:27017/app"', configs["source.yaml"])

    def test_rename_restores_into_the_target_database(self):
        _, restore, configs, _ = commands({"source": SRC, "target": TGT, "options": {}})
        # A database in the restore URI acts as a filter and restored nothing (0 documents).
        self.assertIn('"mongodb://root:p%40ss@tgt:27017/?authSource=app_copy"', configs["target.yaml"])
        self.assertIn("--nsFrom=app.*", restore)
        self.assertIn("--nsTo=app_copy.*", restore)
        self.assertIn("--nsInclude=app.*", restore)

    def test_same_database_is_included_explicitly(self):
        _, restore, configs, _ = commands({"source": SRC, "target": {**TGT, "database": "app"}, "options": {"auth_source": "admin"}})
        self.assertIn("authSource=admin", configs["target.yaml"])
        self.assertEqual(restore[restore.index("--nsInclude") + 1], "app.*")
        self.assertFalse(any(a.startswith("--nsFrom") for a in restore))

    def test_collection_filters_win(self):
        _, restore, _, _ = commands({"source": SRC, "target": TGT, "include_collections": ["users"], "options": {"drop": True}})
        self.assertEqual(restore[restore.index("--nsInclude") + 1], "app.users")
        self.assertIn("--drop", restore)

    def test_cleanup_removes_secrets_and_archive(self):
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp)
            serve.write_run_script(work, {"source": SRC, "target": TGT, "options": {}})
            (work / "dump.archive.gz").write_bytes(b"x")
            serve.cleanup(work)
            self.assertEqual(sorted(p.name for p in work.iterdir()), ["run.sh"])


if __name__ == "__main__":
    unittest.main()
