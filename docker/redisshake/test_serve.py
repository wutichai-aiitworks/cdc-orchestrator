"""Unit tests for the cdc-redisshake sidecar.  Run: cd docker/redisshake && python3 -m unittest -v"""

from __future__ import annotations

import socketserver
import stat
import tempfile
import threading
import unittest
from pathlib import Path

import serve


class FakeRedis(socketserver.StreamRequestHandler):
    """Speaks just enough RESP: AUTH (password "pw"), PING, INFO."""

    def handle(self):
        authed = False
        while True:
            line = self.rfile.readline()
            if not line:
                return
            n = int(line[1:])
            args = []
            for _ in range(n):
                size = int(self.rfile.readline()[1:])
                args.append(self.rfile.read(size + 2)[:-2].decode())
            cmd = args[0].upper()
            if cmd == "AUTH":
                authed = args[-1] == "pw"
                self.wfile.write(b"+OK\r\n" if authed else b"-WRONGPASS invalid username-password pair\r\n")
            elif not authed:
                self.wfile.write(b"-NOAUTH Authentication required.\r\n")
            elif cmd == "PING":
                self.wfile.write(b"+PONG\r\n")
            elif cmd == "INFO":
                body = {"server": "redis_version:7.2.4\r\nredis_mode:standalone\r\n", "keyspace": "db0:keys=3,expires=0\r\ndb2:keys=4,expires=1\r\n",
                        "memory": "used_memory:1048576\r\n"}[args[1]].encode()
                self.wfile.write(b"$%d\r\n%s\r\n" % (len(body), body))


class ProbeTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), FakeRedis)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.port = str(cls.server.server_address[1])

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def probe(self, **kw):
        return serve.test_connection({"connection": {"host": "127.0.0.1", "port": self.port, "password": kw.get("pw", "pw")},
                                      "side": kw.get("side", "source"), "options": kw.get("options", {})})

    def test_reports_version_keys_and_mode(self):
        res = self.probe()
        self.assertTrue(res["ok"], res)
        self.assertEqual(res["facts"][:3], ["Redis 7.2.4", "standalone", "7 keys"])
        self.assertNotIn("warning", res)

    def test_wrong_password(self):
        res = self.probe(pw="nope")
        self.assertFalse(res["ok"])
        self.assertIn("WRONGPASS", res["error"])
        self.assertIn("rejected the login", res["hint"])

    def test_cluster_switch_must_match_and_target_with_keys_warns(self):
        self.assertIn("not a cluster node", self.probe(options={"source_cluster": True})["warning"])
        self.assertIn("already has keys", self.probe(side="target")["warning"])

    def test_refused(self):
        res = serve.test_connection({"connection": {"host": "127.0.0.1", "port": "1"}})
        self.assertFalse(res["ok"])
        self.assertIn("did not accept", res["hint"])


class ConfigTest(unittest.TestCase):
    def test_toml_is_private_and_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp)
            (work / "data").mkdir()
            path = serve.write_shake_toml(work, {"source": {"host": "a", "password": "p1"}, "target": {"host": "b", "password": "p2"}, "options": {}})
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            serve.cleanup(work)
            self.assertEqual(list(work.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
