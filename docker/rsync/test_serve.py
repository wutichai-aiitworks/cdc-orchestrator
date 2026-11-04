"""Unit tests for the cdc-rsync sidecar.  Run: cd docker/rsync && python3 -m unittest -v"""

from __future__ import annotations

import base64
import hashlib
import shlex
import subprocess
import unittest

import serve

KEY = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n"
SRC = {"host": "src.lan", "username": "deploy", "auth": "key", "private_key": KEY, "path": "/srv/www/"}
TGT = {"host": "tgt.example.com", "port": "2222", "username": "backup", "auth": "key", "private_key": KEY,
       "path": "/data/www", "reach_host": "10.0.0.9"}
GNU = serve.rsync_flavor("rsync  version 3.2.7  protocol version 31")
OLD = serve.rsync_flavor("rsync  version 2.6.9  protocol version 29")
OPENRSYNC = serve.rsync_flavor("openrsync: protocol version 29")


def req(**options):
    return serve.validate_request({"source": SRC, "target": TGT, "options": options})


class ValidateTest(unittest.TestCase):
    def test_defaults(self):
        r = req()
        self.assertEqual(r["transport"], "auto")
        self.assertEqual(r["source"]["port"], 22)
        self.assertEqual(r["target"]["port"], 2222)
        self.assertFalse(r["flags"]["delete"])
        self.assertFalse(r["flags"]["dry_run"])

    def test_unknown_options_rejected_but_unset_ignored(self):
        with self.assertRaises(serve.BadRequest):
            req(exclude=["*.log"])
        r = serve.validate_request({"source": SRC, "target": TGT, "options": {"exclude": None, "foo": False}})
        self.assertEqual(r["excludes"], [])

    def test_direct_needs_key_target(self):
        pw_target = {**TGT, "auth": "password", "password": "x", "private_key": ""}
        with self.assertRaises(serve.BadRequest):
            serve.validate_request({"source": SRC, "target": pw_target, "options": {"transport": "direct"}})
        r = serve.validate_request({"source": SRC, "target": pw_target, "options": {"transport": "auto"}})
        self.assertEqual(r["target"]["auth"], "password")

    def test_endpoint_checks(self):
        for bad in ({"host": "-oProxyCommand=x"}, {"username": "a b"}, {"path": "-rf"}, {"path": ""},
                    {"host_key_fingerprint": "MD5:aa"}, {"rsync_path": "rsync"}, {"port": "0"},
                    {"auth": "key", "private_key": "nope"}, {"path": "a\nb"}, {"reach_host": "-x"}):
            with self.assertRaises(serve.BadRequest, msg=str(bad)):
                serve.norm_endpoint({**SRC, **bad}, "source")

    def test_tilde_paths_become_home_relative(self):
        self.assertEqual(serve.norm_endpoint({**SRC, "path": "~/site"}, "source")["path"], "site")
        self.assertEqual(serve.norm_endpoint({**SRC, "path": "~/"}, "source")["path"], ".")

    def test_password_endpoint_drops_key_fields(self):
        e = serve.norm_endpoint({**SRC, "auth": "password", "password": "pw", "passphrase": "x"}, "source")
        self.assertEqual((e["password"], e["private_key"], e["passphrase"]), ("pw", "", ""))

    def test_max_delete_needs_delete_and_ints_are_ranged(self):
        with self.assertRaises(serve.BadRequest):
            req(max_delete=10)
        self.assertEqual(req(delete=True, max_delete=10)["ints"]["max_delete"], 10)
        with self.assertRaises(serve.BadRequest):
            req(bwlimit_kbps=0)
        with self.assertRaises(serve.BadRequest):
            req(io_timeout_seconds=True)

    def test_run_level_dry_run_overrides_profile(self):
        body = {"source": SRC, "target": TGT, "options": {"dry_run": False}, "dry_run": True}
        self.assertTrue(serve.validate_request(body)["flags"]["dry_run"])

    def test_excludes(self):
        self.assertEqual(req(excludes="*.log, .git/ ,")["excludes"], ["*.log", ".git/"])
        with self.assertRaises(serve.BadRequest):
            req(excludes=["a\nb"])


class HostKeyTest(unittest.TestCase):
    BLOB = base64.b64encode(b"\x00\x00\x00\x0bssh-ed25519" + b"k" * 32).decode()

    def test_fingerprint_matches_openssh_format(self):
        want = "SHA256:" + base64.b64encode(hashlib.sha256(base64.b64decode(self.BLOB)).digest()).decode().rstrip("=")
        self.assertEqual(serve.key_fingerprint(self.BLOB), want)
        self.assertTrue(serve.FINGERPRINT.match(want))

    def test_parse_keyscan_and_accept(self):
        text = f"# comment\n[h]:2222 ssh-ed25519 {self.BLOB}\nh ssh-rsa !!notbase64\n"
        keys = serve.parse_keyscan(text)
        self.assertEqual(len(keys), 1)
        fp = keys[0]["fingerprint"]
        trusted, shown, pinned = serve.accept_keys(keys, "", "t")
        self.assertEqual((shown, pinned, len(trusted)), (fp, False, 1))
        trusted, shown, pinned = serve.accept_keys(keys, fp, "t")
        self.assertTrue(pinned)
        with self.assertRaises(serve.StepFailed) as cm:
            serve.accept_keys(keys, "SHA256:" + "A" * 43, "t")
        self.assertIn("mismatch", str(cm.exception))
        with self.assertRaises(serve.StepFailed):
            serve.accept_keys([], "", "t")

    def test_unpinned_reports_preferred_type(self):
        rsa = {"type": "ssh-rsa", "key": "x", "fingerprint": "SHA256:rsa"}
        ed = {"type": "ssh-ed25519", "key": "y", "fingerprint": "SHA256:ed"}
        self.assertEqual(serve.accept_keys([rsa, ed], "", "t")[1], "SHA256:ed")

    def test_known_hosts_uses_alias(self):
        line = serve.known_hosts_lines("cdc-rsync-target", [{"type": "ssh-ed25519", "key": "AAA"}])
        self.assertEqual(line, "cdc-rsync-target ssh-ed25519 AAA\n")


class FlavorTest(unittest.TestCase):
    def test_versions(self):
        self.assertTrue(GNU["gnu"] and GNU["progress"] and GNU["protect_args"])
        self.assertFalse(OLD["gnu"])
        self.assertFalse(OPENRSYNC["gnu"])
        self.assertTrue(OPENRSYNC["openrsync"])
        old_gnu = serve.rsync_flavor("rsync  version 3.0.9  protocol version 30")
        self.assertTrue(old_gnu["gnu"] and not old_gnu["progress"])

    def test_parse_probe_prefers_gnu(self):
        text = ("os=Darwin\narch=arm64\nrsync=/usr/bin/rsync|openrsync: protocol version 29\n"
                "rsync=/opt/homebrew/bin/rsync|rsync  version 3.3.0  protocol version 31\n"
                "kind=dir\nfree_kb=100\nwritable=1\nssh=1\nprobe=ok\n")
        p = serve.parse_probe(text)
        self.assertEqual(p["rsync"]["path"], "/opt/homebrew/bin/rsync")
        self.assertEqual((p["os"], p["kind"], p["free_bytes"], p["writable"], p["ssh"]), ("Darwin", "dir", 102400, True, True))

    def test_rsync_hints(self):
        self.assertIn("brew", serve.rsync_hint({"os": "Darwin", "rsync": None}))
        self.assertIn("apt", serve.rsync_hint({"os": "Linux", "rsync": None}))
        self.assertIn("/opt/homebrew", serve.rsync_hint({"arch": "arm64", "rsync": {**OPENRSYNC, "path": "/usr/bin/rsync"}}))
        self.assertIsNone(serve.rsync_hint({"rsync": {**GNU, "path": "/usr/bin/rsync"}}))

    def test_probe_script_runs_in_sh(self):
        out = subprocess.run(["sh", "-c", serve.probe_script("/definitely/missing/dir it's")],
                             capture_output=True, text=True, timeout=10)
        self.assertEqual(out.returncode, 0, out.stderr)
        p = serve.parse_probe(out.stdout)
        self.assertEqual(p["kind"], "missing")
        self.assertEqual(p["probe"], "ok")


class CommandTest(unittest.TestCase):
    def test_flags(self):
        r = req(delete=True, max_delete=5, compress=True, no_owner=True, bwlimit_kbps=500, excludes=["*.tmp"])
        f = serve.rsync_flags(r, GNU, GNU)
        for want in ("--archive", "--stats", "--info=progress2", "--protect-args", "--delete", "--max-delete=5",
                     "--compress", "--no-owner", "--no-group", "--bwlimit=500", "--exclude=*.tmp"):
            self.assertIn(want, f)
        self.assertNotIn("--dry-run", f)

    def test_old_client_or_server_drops_new_flags(self):
        f = serve.rsync_flags(req(), OLD, GNU)
        self.assertNotIn("--info=progress2", f)
        self.assertNotIn("--protect-args", f)
        self.assertNotIn("--protect-args", serve.rsync_flags(req(), GNU, OPENRSYNC))

    def test_dry_run_itemizes_and_mirror(self):
        f = serve.rsync_flags(req(dry_run=True), GNU, GNU)
        self.assertIn("--itemize-changes", f)
        m = serve.rsync_flags(req(max_delete=1, delete=True), GNU, GNU, delete=True, dry_run=False, mirror=True)
        self.assertIn("--delete-excluded", m)
        self.assertNotIn("--max-delete=1", m)  # staging mirror is ours, the limit guards the target

    def test_compat(self):
        r = req(acls=True)
        with self.assertRaises(serve.StepFailed):
            serve.check_compat(r, {"source": GNU, "target": OPENRSYNC})
        serve.check_compat(r, {"source": GNU, "target": GNU})
        spaced = serve.validate_request({"source": {**SRC, "path": "/srv/my site"}, "target": TGT})
        with self.assertRaises(serve.StepFailed):
            serve.check_compat(spaced, {"source": OLD, "target": GNU})
        serve.check_compat(spaced, {"source": GNU, "target": GNU})

    def test_destinations(self):
        r = req()
        self.assertEqual(serve.remote_dest(r["target"]), "backup@tgt.example.com:/data/www/")
        self.assertEqual(serve.remote_dest(r["target"], via_reach=True), "backup@10.0.0.9:/data/www/")
        v6 = serve.norm_endpoint({**TGT, "host": "fd00::1", "reach_host": ""}, "target")
        self.assertEqual(serve.remote_dest(v6), "backup@[fd00::1]:/data/www/")
        root = serve.norm_endpoint({**TGT, "path": "/"}, "target")
        self.assertEqual(serve.remote_dest(root), "backup@tgt.example.com://")

    def test_rsync_path_creates_target_dir(self):
        t = serve.norm_endpoint({**TGT, "path": "/data/it's here"}, "target")
        arg = serve.target_rsync_path(t, "/opt/homebrew/bin/rsync")
        self.assertTrue(arg.startswith("--rsync-path=mkdir -p "))
        cmd = arg.split("=", 1)[1]
        self.assertEqual(shlex.split(cmd), ["mkdir", "-p", "/data/it's here", "&&", "/opt/homebrew/bin/rsync"])

    def test_direct_script_is_valid_sh_and_quotes_everything(self):
        r = serve.validate_request({"source": {**SRC, "path": "/srv/a b;rm -rf ~"}, "target": TGT,
                                    "options": {"excludes": ["$(touch /tmp/pwn)"]}})
        src = {**GNU, "path": "/usr/bin/rsync"}
        tgt = {**GNU, "path": "/opt/homebrew/bin/rsync"}
        script = serve.direct_script(r, "abcd1234", "cdc-rsync-target ssh-ed25519 AAA\n", src, tgt)
        self.assertEqual(subprocess.run(["sh", "-n", "-c", script]).returncode, 0)
        self.assertIn("'/srv/a b;rm -rf ~'", script)
        self.assertIn("'--exclude=$(touch /tmp/pwn)'", script)
        self.assertIn("HostKeyAlias=cdc-rsync-target", script)
        self.assertIn("-p 2222", script)
        self.assertIn("backup@10.0.0.9:/data/www/", script)
        self.assertIn("cdc-rsync-abcd1234.XXXXXX", script)

    def test_hop_prelude_writes_known_hosts_and_cleans_up(self):
        lines = "cdc-rsync-target ssh-ed25519 AAA'B\n"
        script = serve.hop_prelude("t1", lines) + 'cat "$kh"; echo "$kh" >&2\n'
        out = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=10)
        self.assertEqual(out.stdout, lines)
        self.assertIn("cdc-rsync-t1.", out.stderr)
        self.assertFalse(subprocess.run(["test", "-e", out.stderr.strip()]).returncode == 0)

    def test_hop_probe_nests_quoting(self):
        r = serve.validate_request({"source": SRC, "target": {**TGT, "path": "/x'y"}})
        script = serve.hop_probe_script(r, "t", "k\n")
        self.assertEqual(subprocess.run(["sh", "-n", "-c", script]).returncode, 0)
        self.assertIn("backup@10.0.0.9", script)

    def test_ssh_auth(self):
        e = serve.norm_endpoint(SRC, "source")
        prefix, opts = serve.ssh_auth(e, "/s/agent.sock")
        self.assertEqual(prefix, [])
        self.assertIn("IdentityAgent=/s/agent.sock", opts)
        pw = serve.norm_endpoint({**SRC, "auth": "password", "password": "pw"}, "source")
        prefix, opts = serve.ssh_auth(pw, None)
        self.assertEqual(prefix, ["sshpass", "-e"])
        self.assertNotIn("pw", " ".join(opts))


class OutputTest(unittest.TestCase):
    def test_progress(self):
        p = serve.parse_progress("    1,234,567  45%   10.20MB/s    0:00:12 (xfr#12, to-chk=3/20)")
        self.assertEqual((p["bytes"], p["percent"], p["files_transferred"], p["files_total"]), (1234567, 45, 12, 20))
        self.assertEqual(serve.parse_progress("  999 100%  1.00kB/s 0:00:00")["percent"], 100)
        self.assertIsNone(serve.parse_progress("sending incremental file list"))

    def test_stats(self):
        text = ("Number of files: 1,234 (reg: 1,000, dir: 234)\nNumber of created files: 5\n"
                "Number of deleted files: 2\nNumber of regular files transferred: 7\n"
                "Total file size: 12,345 bytes\nTotal transferred file size: 1,234 bytes\n"
                "Literal data: 1,000 bytes\nTotal bytes sent: 2,048\nsent 2,048 bytes  received 35 bytes\n")
        s = serve.parse_stats(text)
        self.assertEqual(s["number_of_files"], 1234)
        self.assertEqual(s["number_of_regular_files_transferred"], 7)
        self.assertEqual(s["number_of_deleted_files"], 2)
        self.assertEqual(s["total_transferred_file_size"], 1234)
        self.assertEqual(s["total_bytes_sent"], 2048)

    def test_items(self):
        self.assertEqual(serve.parse_item(">f+++++++++ docs/a.txt"), {"op": "create", "kind": "file", "path": "docs/a.txt"})
        self.assertEqual(serve.parse_item("<f.st...... b.bin")["op"], "update")
        self.assertEqual(serve.parse_item("cd+++++++++ newdir/")["op"], "create")
        self.assertEqual(serve.parse_item("*deleting   old/x.log"), {"op": "delete", "kind": "file", "path": "old/x.log"})
        self.assertEqual(serve.parse_item(".d..t...... ./")["op"], "attrs")
        self.assertIsNone(serve.parse_item("Number of files: 3"))

    def test_outcome(self):
        self.assertEqual(serve.outcome(0, False), ("succeeded", None))
        self.assertEqual(serve.outcome(24, False)[0], "completed_with_errors")
        self.assertEqual(serve.outcome(25, False)[0], "completed_with_errors")
        self.assertEqual(serve.outcome(255, False)[0], "failed")
        self.assertEqual(serve.outcome(255, True), ("stopped", None))

    def test_hints(self):
        self.assertIn("host key", serve.hint_for("Host key verification failed.").lower())
        self.assertIn("rejected", serve.hint_for("Permission denied (publickey,password)."))
        self.assertIn("rejected", serve.hint_for("source: Permission denied (wrong password)"))
        self.assertIn("missing", serve.hint_for("bash: rsync: command not found"))
        self.assertIn("max_delete", serve.hint_for("Deletions stopped due to --max-delete limit (3 skipped)"))
        self.assertIn("max_delete", serve.hint_for("rsync error: the --max-delete limit stopped deletions (code 25)"))
        self.assertIn("direct", serve.hint_for("ssh: connect to host x port 22: Connection refused"))
        self.assertIsNone(serve.hint_for("all good"))

    def test_first_error(self):
        log = "sending incremental file list\nrsync: [sender] send_files failed to open \"/a\": Permission denied (13)\n"
        self.assertIn("send_files", serve.first_error(log))


if __name__ == "__main__":
    unittest.main()
