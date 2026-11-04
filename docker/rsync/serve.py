#!/usr/bin/env python3
"""HTTP sidecar for host-to-host file sync with rsync over SSH (Linux and macOS hosts).

Transports (ADR 0007):
  direct  The sidecar SSHes into the source and runs rsync there, pushing straight to the target.
          The target key travels only in a forwarded ssh-agent (never written on the source).
  relay   The sidecar pulls source → staging volume, then pushes staging → target. Used when the
          source cannot reach the target, or the target only accepts a password.
  auto    (default) Try the direct route first; fall back to relay and record why.

Same job contract as the other sidecars (/health, /v1/discover, /v1/capability, /v1/jobs …) plus
POST /v1/test-connection, POST /v1/test-route and /v1/staging.

Secrets: never on argv, in job.json or in logs. Private keys are loaded into per-job ssh-agents
(passphrase via an SSH_ASKPASS script reading the environment) and the key files are deleted at
once; passwords reach sshpass through SSHPASS. The per-job .secrets directory is removed when the
job ends and on sidecar start.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import tempfile
import threading
import time
import uuid
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse

WORK_ROOT = Path(os.environ.get("RSYNC_WORK_ROOT", "/var/lib/rsync"))
JOBS_DIR = WORK_ROOT / "jobs"
STAGING_DIR = WORK_ROOT / "staging"
API_TOKEN = os.environ.get("RSYNC_API_TOKEN", "").strip()
HOST = os.environ.get("RSYNC_API_HOST", "0.0.0.0")
PORT = int(os.environ.get("RSYNC_API_PORT", "8090"))
MAX_CONCURRENT = int(os.environ.get("RSYNC_MAX_CONCURRENT", "2"))
MIN_FREE_MB = int(os.environ.get("RSYNC_MIN_FREE_MB", "1024"))
TIMEOUT_SECONDS = int(os.environ.get("RSYNC_TIMEOUT_SECONDS", "0"))  # 0 = no limit
RETENTION_HOURS = int(os.environ.get("RSYNC_RETENTION_HOURS", "168"))
STAGING_RETENTION_DAYS = int(os.environ.get("RSYNC_STAGING_RETENTION_DAYS", "14"))
SSH_TIMEOUT = int(os.environ.get("RSYNC_SSH_TIMEOUT_SECONDS", "15"))
RSYNC = os.environ.get("RSYNC_BIN", "rsync")
SSH = os.environ.get("RSYNC_SSH_BIN", "ssh")

TRANSPORTS = ("auto", "direct", "relay")
ACTIVE = ("starting", "running")
SOURCE_ALIAS = "cdc-rsync-source"
TARGET_ALIAS = "cdc-rsync-target"
# Homebrew rsync first: macOS 15 ships openrsync as /usr/bin/rsync (no --info, -A, -X), and a
# non-interactive SSH session on macOS does not have /opt/homebrew/bin on PATH.
RSYNC_CANDIDATES = ("/opt/homebrew/bin/rsync", "/usr/local/bin/rsync", "/usr/bin/rsync", "rsync")

BOOL_FLAGS: dict[str, list[str]] = {
    "checksum": ["--checksum"],
    "compress": ["--compress"],
    "no_owner": ["--no-owner", "--no-group"],
    "hard_links": ["--hard-links"],
    "acls": ["--acls"],
    "xattrs": ["--xattrs"],
    "numeric_ids": ["--numeric-ids"],
    "one_file_system": ["--one-file-system"],
}
OTHER_BOOLS = ("delete", "dry_run", "discard_staging")
INT_OPTIONS = {"bwlimit_kbps": (1, 10_000_000), "max_delete": (0, 100_000_000), "io_timeout_seconds": (10, 86_400)}
KNOWN_OPTIONS = set(BOOL_FLAGS) | set(OTHER_BOOLS) | set(INT_OPTIONS) | {"excludes", "transport"}
GNU_ONLY = ("acls", "xattrs")

JOB_ID = re.compile(r"^[A-Za-z0-9-]{1,64}$")
HOST_RE = re.compile(r"^[A-Za-z0-9._:\-\[\]]{1,253}$")
USER_RE = re.compile(r"^[A-Za-z0-9._][A-Za-z0-9._-]{0,63}$")
BIN_RE = re.compile(r"^/[A-Za-z0-9._/+-]{1,255}$")
FINGERPRINT = re.compile(r"^SHA256:[A-Za-z0-9+/]{43}=?$")
# Paths every rsync version passes through the remote shell safely without --protect-args.
PLAIN_PATH = re.compile(r"^[A-Za-z0-9._/@+,=:-]+$")

_lock = threading.Lock()
_procs: dict[str, subprocess.Popen[Any]] = {}
_runtime: dict[str, dict[str, Any]] = {}
_stop_requested: set[str] = set()
_stoppers: dict[str, Any] = {}


class BadRequest(Exception):
    def __init__(self, message: str, code: int = 400) -> None:
        super().__init__(message)
        self.code = code


class StepFailed(Exception):
    """A preflight step failed with a message meant for the operator."""

    def __init__(self, message: str, hint: str | None = None) -> None:
        super().__init__(message)
        self.hint = hint


# ── validation (unit-tested) ──────────────────────────────────────────────────


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def _clean_path(raw: Any, where: str) -> str:
    path = str(raw or "").strip()
    if path.startswith("~/"):
        path = path[2:] or "."  # rsync and sh both resolve relative paths from the login home
    if not path or path == "~":
        raise BadRequest(f"{where} is required (absolute, or relative to the login's home)")
    if len(path) > 4096 or any(c in path for c in "\r\n\0"):
        raise BadRequest(f"{where} contains invalid characters")
    if path.startswith("-"):
        raise BadRequest(f"{where} must not start with '-'")
    return path


def norm_endpoint(raw: Any, role: str) -> dict[str, Any]:
    """Validate one SSH host (source or target). Secrets stay in memory only."""
    if not isinstance(raw, dict) or not raw:
        raise BadRequest(f"{role} is required")
    host = str(raw.get("host") or "").strip()
    if not host or not HOST_RE.match(host) or host.startswith("-"):
        raise BadRequest(f"{role}.host is required and must be a hostname or IP")
    port = str(raw.get("port") or "22").strip()
    if not port.isdigit() or not 1 <= int(port) <= 65535:
        raise BadRequest(f"{role}.port must be 1-65535")
    user = str(raw.get("username") or "").strip()
    if not USER_RE.match(user):
        raise BadRequest(f"{role}.username is required (letters, digits, . _ -)")
    password = str(raw.get("password") or "")
    private_key = str(raw.get("private_key") or "")
    auth = str(raw.get("auth") or ("key" if private_key else "password")).strip()
    if auth not in ("key", "password"):
        raise BadRequest(f"{role}.auth must be 'key' or 'password'")
    if auth == "key" and "PRIVATE KEY" not in private_key:
        raise BadRequest(f"{role}.private_key is required for auth=key (OpenSSH or PEM format)")
    if auth == "password" and not password:
        raise BadRequest(f"{role}.password is required for auth=password")
    fp = str(raw.get("host_key_fingerprint") or "").strip()
    if fp and not FINGERPRINT.match(fp):
        raise BadRequest(f"{role}.host_key_fingerprint must look like SHA256:<43 base64 chars> (ssh-keygen -lf)")
    rsync_path = str(raw.get("rsync_path") or "").strip()
    if rsync_path and not BIN_RE.match(rsync_path):
        raise BadRequest(f"{role}.rsync_path must be an absolute path such as /opt/homebrew/bin/rsync")
    reach_host = str(raw.get("reach_host") or "").strip()
    if reach_host and (not HOST_RE.match(reach_host) or reach_host.startswith("-")):
        raise BadRequest(f"{role}.reach_host must be a hostname or IP")
    reach_port = str(raw.get("reach_port") or "").strip()
    if reach_port and (not reach_port.isdigit() or not 1 <= int(reach_port) <= 65535):
        raise BadRequest(f"{role}.reach_port must be 1-65535")
    return {
        "name": str(raw.get("name") or host).strip()[:60],
        "host": host, "port": int(port), "username": user, "auth": auth,
        "password": password if auth == "password" else "",
        "private_key": private_key if auth == "key" else "",
        "passphrase": str(raw.get("passphrase") or "") if auth == "key" else "",
        "path": _clean_path(raw.get("path"), f"{role}.path"),
        "host_key_fingerprint": fp.rstrip("="),
        "rsync_path": rsync_path,
        # How the *source* reaches this host in direct mode (LAN address, other port).
        "reach_host": reach_host,
        "reach_port": int(reach_port) if reach_port else None,
    }


def describe_endpoint(e: dict[str, Any]) -> str:
    """Credential-free description for job status."""
    return f"{e['username']}@{e['host']}:{e['port']}:{e['path']}"


def _int_opt(options: dict[str, Any], key: str) -> int | None:
    raw = options.get(key)
    if raw is None or raw == "":
        return None
    lo, hi = INT_OPTIONS[key]
    if isinstance(raw, bool):
        raise BadRequest(f"options.{key} must be an integer")
    try:
        val = int(raw)
    except (TypeError, ValueError):
        raise BadRequest(f"options.{key} must be an integer") from None
    if not lo <= val <= hi:
        raise BadRequest(f"options.{key} must be between {lo} and {hi}")
    return val


def _excludes(raw: Any) -> list[str]:
    if raw in (None, "", []):
        return []
    if isinstance(raw, str):
        raw = raw.split(",")
    if not isinstance(raw, list):
        raise BadRequest("options.excludes must be a list of patterns")
    if len(raw) > 200:
        raise BadRequest("options.excludes accepts at most 200 patterns")
    out = []
    for item in raw:
        s = str(item).strip()
        if not s:
            continue
        if len(s) > 512 or any(c in s for c in "\r\n\0"):
            raise BadRequest(f"options.excludes entry {s[:40]!r} is not a valid pattern")
        out.append(s)
    return out


def validate_request(body: dict[str, Any]) -> dict[str, Any]:
    """Normalise a POST /v1/jobs body. Unknown option keys are rejected: a typo must never
    silently widen a sync (e.g. 'exclude' instead of 'excludes')."""
    options = body.get("options") or {}
    if not isinstance(options, dict):
        raise BadRequest("options must be an object")
    unknown = sorted(k for k, v in options.items() if k not in KNOWN_OPTIONS and v not in (None, False, "", []))
    if unknown:
        raise BadRequest(f"unknown options: {unknown}")
    transport = str(options.get("transport") or "auto").strip()
    if transport not in TRANSPORTS:
        raise BadRequest(f"options.transport must be one of {list(TRANSPORTS)}")
    source = norm_endpoint(body.get("source"), "source")
    target = norm_endpoint(body.get("target"), "target")
    if transport == "direct" and target["auth"] != "key":
        raise BadRequest("transport=direct needs key auth on the target (the key is forwarded through the "
                         "source's SSH session); use auto or relay for password targets")
    flags = {k: bool(options.get(k)) for k in list(BOOL_FLAGS) + list(OTHER_BOOLS)}
    ints = {k: _int_opt(options, k) for k in INT_OPTIONS}
    if ints["max_delete"] is not None and not flags["delete"]:
        raise BadRequest("options.max_delete requires options.delete")
    dry = body.get("dry_run")
    if dry is not None:
        flags["dry_run"] = bool(dry)  # per-run override ("Preview changes")
    profile_id = str(body.get("profile_id") or "").strip()[:64] or None
    if profile_id and not JOB_ID.match(profile_id):
        raise BadRequest("profile_id must be letters, digits and dashes")
    return {
        "name": str(body.get("name") or "").strip()[:120],
        "profile_id": profile_id,
        "source": source,
        "target": target,
        "transport": transport,
        "flags": flags,
        "ints": ints,
        "excludes": _excludes(options.get("excludes")),
    }


# ── SSH host keys (unit-tested) ───────────────────────────────────────────────


def key_fingerprint(b64_blob: str) -> str:
    """OpenSSH SHA256 fingerprint (ssh-keygen -lf) of a base64 public key, without padding."""
    digest = hashlib.sha256(base64.b64decode(b64_blob)).digest()
    return "SHA256:" + base64.b64encode(digest).decode().rstrip("=")


def parse_keyscan(text: str) -> list[dict[str, str]]:
    """`ssh-keyscan` lines ("host type base64", host may be "[h]:port") → [{type, key, fingerprint}]."""
    out = []
    for line in text.splitlines():
        parts = line.strip().split()
        if len(parts) < 3 or line.startswith("#"):
            continue
        ktype, blob = parts[1], parts[2]
        try:
            fp = key_fingerprint(blob)
        except (ValueError, base64.binascii.Error):
            continue
        out.append({"type": ktype, "key": blob, "fingerprint": fp})
    return out


KEY_PREFERENCE = ("ssh-ed25519", "ecdsa-sha2-nistp256", "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp521", "ssh-rsa")


def accept_keys(scanned: list[dict[str, str]], pinned: str, label: str) -> tuple[list[dict[str, str]], str, bool]:
    """Keys to trust for this job, the fingerprint to report, and whether it was pinned.

    Pinned: only the matching key is trusted (another key type the server offers is then rejected
    by StrictHostKeyChecking). Unpinned: every scanned key, reported for one-click pinning.
    """
    if not scanned:
        raise StepFailed(f"{label}: no SSH host key received", "The host did not answer on the SSH port; check host, port and firewall.")
    if pinned:
        match = [k for k in scanned if k["fingerprint"] == pinned]
        if not match:
            seen = ", ".join(sorted({k["fingerprint"] for k in scanned}))
            raise StepFailed(f"{label}: host key mismatch: expected {pinned}, got {seen}",
                             "The host key differs from the pinned fingerprint: the server was reinstalled or "
                             "this is not the expected host. Verify, then update the fingerprint.")
        return match, pinned, True
    ranked = sorted(scanned, key=lambda k: KEY_PREFERENCE.index(k["type"]) if k["type"] in KEY_PREFERENCE else 99)
    return scanned, ranked[0]["fingerprint"], False


def known_hosts_lines(alias: str, keys: list[dict[str, str]]) -> str:
    return "".join(f"{alias} {k['type']} {k['key']}\n" for k in keys)


# ── remote rsync detection (unit-tested) ──────────────────────────────────────

VERSION_RE = re.compile(r"rsync\s+version\s+v?(\d+)\.(\d+)(?:\.(\d+))?")


def rsync_flavor(version_line: str) -> dict[str, Any]:
    """'rsync  version 3.2.7  protocol version 31' → GNU 3.2; 'openrsync: protocol version 29' → openrsync."""
    line = version_line.strip()
    m = VERSION_RE.search(line)
    openrsync = "openrsync" in line.lower()
    ver = (int(m.group(1)), int(m.group(2))) if m else None
    gnu = bool(ver) and not openrsync and ver >= (3, 0)
    return {
        "version": line[:120],
        "gnu": gnu,
        "openrsync": openrsync,
        "major_minor": f"{ver[0]}.{ver[1]}" if ver else None,
        "progress": gnu and ver >= (3, 1),  # --info=progress2,stats2
        "protect_args": gnu,               # -s (3.0+)
    }


def probe_script(path: str, rsync_path: str = "") -> str:
    """POSIX sh run on a host: OS, rsync binaries, path state, free space, ssh client."""
    candidates = (rsync_path,) if rsync_path else RSYNC_CANDIDATES
    cands = " ".join(shlex.quote(c) for c in candidates)
    return (
        f"p={shlex.quote(path)}\n"
        'echo "os=$(uname -s 2>/dev/null)"\n'
        'echo "release=$(uname -r 2>/dev/null)"\n'
        'echo "arch=$(uname -m 2>/dev/null)"\n'
        f"for r in {cands}; do\n"
        '  if [ -x "$r" ] || { [ "${r#/}" = "$r" ] && command -v "$r" >/dev/null 2>&1; }; then\n'
        '    echo "rsync=$r|$("$r" --version 2>&1 | head -n 1)"\n'
        "  fi\n"
        "done\n"
        'if [ -d "$p" ]; then echo kind=dir; elif [ -e "$p" ]; then echo kind=file; else echo kind=missing; fi\n'
        'd="$p"; while [ ! -e "$d" ]; do n=$(dirname "$d"); [ "$n" = "$d" ] && break; d="$n"; done\n'
        'df -Pk "$d" 2>/dev/null | awk \'NR==2 {print "free_kb=" $4}\'\n'
        'if [ -w "$d" ]; then echo writable=1; else echo writable=0; fi\n'
        'if command -v ssh >/dev/null 2>&1; then echo ssh=1; else echo ssh=0; fi\n'
        'echo probe=ok\n'
    )


def parse_probe(text: str) -> dict[str, Any]:
    out: dict[str, Any] = {"rsync_candidates": []}
    for line in text.splitlines():
        key, _, value = line.partition("=")
        if key == "rsync" and "|" in value:
            path, _, version = value.partition("|")
            out["rsync_candidates"].append({"path": path, **rsync_flavor(version)})
        elif key == "free_kb" and value.strip().isdigit():
            out["free_bytes"] = int(value) * 1024
        elif key in ("writable", "ssh"):
            out[key] = value.strip() == "1"
        elif key in ("os", "release", "arch", "kind", "probe"):
            out[key] = value.strip()
    out["rsync"] = pick_rsync(out["rsync_candidates"])
    return out


def pick_rsync(cands: list[dict[str, Any]]) -> dict[str, Any] | None:
    """Prefer GNU ≥3.1 (progress, -A/-X), then any GNU, then whatever exists (openrsync, 2.6.9)."""
    for test in (lambda c: c["progress"], lambda c: c["gnu"], lambda c: True):
        for c in cands:
            if test(c):
                return c
    return None


def rsync_hint(probe: dict[str, Any]) -> str | None:
    r = probe.get("rsync")
    if r is None:
        if probe.get("os") == "Darwin":
            return "rsync not found. Install it with Homebrew (brew install rsync) or set the rsync path."
        return "rsync is not installed on this host (apt install rsync / dnf install rsync)."
    if not r["gnu"]:
        brew = "/opt/homebrew/bin/rsync" if probe.get("arch") == "arm64" else "/usr/local/bin/rsync"
        return (f"{r['path']} is {'openrsync' if r['openrsync'] else 'an old rsync'}: no live progress, ACLs or "
                f"xattrs. Install GNU rsync (brew install rsync); it is picked up from {brew} automatically.")
    return None


# ── rsync command lines (unit-tested) ────────────────────────────────────────


def rsync_flags(req: dict[str, Any], client: dict[str, Any], remote: dict[str, Any] | None,
                *, delete: bool | None = None, dry_run: bool | None = None, mirror: bool = False) -> list[str]:
    """rsync options for one transfer. `client` runs the command, `remote` is the other side."""
    f, ints = req["flags"], req["ints"]
    delete = f["delete"] if delete is None else delete
    dry_run = f["dry_run"] if dry_run is None else dry_run
    argv = ["--archive", "--partial-dir=.rsync-partial", "--stats"]
    if client.get("progress"):
        argv += ["--info=progress2", "--no-human-readable"]
    if client.get("protect_args") and (remote is None or remote.get("protect_args")):
        argv.append("--protect-args")
    if dry_run:
        argv += ["--dry-run", "--itemize-changes"]
    if delete:
        argv.append("--delete")
        if mirror:
            argv.append("--delete-excluded")
        elif ints["max_delete"] is not None:
            argv.append(f"--max-delete={ints['max_delete']}")
    for key, flags in BOOL_FLAGS.items():
        if f[key]:
            argv += flags
    if ints["bwlimit_kbps"]:
        argv.append(f"--bwlimit={ints['bwlimit_kbps']}")
    if ints["io_timeout_seconds"]:
        argv.append(f"--timeout={ints['io_timeout_seconds']}")
    argv += [f"--exclude={p}" for p in req["excludes"]]
    return argv


def check_compat(req: dict[str, Any], sides: dict[str, dict[str, Any] | None]) -> None:
    """Fail early on options a side's rsync cannot do, and on paths that need --protect-args."""
    for opt in GNU_ONLY:
        if req["flags"][opt]:
            for role, r in sides.items():
                if r is not None and not r["gnu"]:
                    raise StepFailed(f"options.{opt} needs GNU rsync on both hosts; {role} has {r['version']}",
                                     "Install GNU rsync there (brew install rsync on macOS) or turn the option off.")
    if not all(r is not None and r["protect_args"] for r in sides.values()):
        for role in ("source", "target"):
            path = req[role]["path"]
            if not PLAIN_PATH.match(path):
                raise StepFailed(f"{role}.path has spaces or shell characters and an rsync without "
                                 "--protect-args is involved", "Install GNU rsync 3.x on both hosts, or use a plain path.")


def host_spec(host: str) -> str:
    return f"[{host}]" if ":" in host and not host.startswith("[") else host


def remote_dest(e: dict[str, Any], via_reach: bool = False) -> str:
    host = (e["reach_host"] or e["host"]) if via_reach else e["host"]
    return f"{e['username']}@{host_spec(host)}:{e['path'].rstrip('/') or '/'}/"


def target_rsync_path(target: dict[str, Any], rsync_bin: str) -> str:
    """--rsync-path that creates the target directory first: rsync then always treats it as a dir."""
    return f"--rsync-path=mkdir -p {shlex.quote(target['path'])} && {shlex.quote(rsync_bin)}"


def ssh_base(port: int, known_hosts: str, alias: str) -> list[str]:
    return ["-F", "/dev/null", "-p", str(port),
            "-o", "StrictHostKeyChecking=yes", "-o", f"UserKnownHostsFile={known_hosts}",
            "-o", "GlobalKnownHostsFile=/dev/null", "-o", f"HostKeyAlias={alias}",
            "-o", "UpdateHostKeys=no", "-o", f"ConnectTimeout={SSH_TIMEOUT}",
            "-o", "ServerAliveInterval=30", "-o", "ServerAliveCountMax=4", "-o", "LogLevel=ERROR"]


def ssh_auth(e: dict[str, Any], agent_sock: str | None) -> tuple[list[str], list[str]]:
    """(prefix, options) for logging in to `e` from the sidecar. Password → sshpass -e (SSHPASS env)."""
    if e["auth"] == "key":
        return [], ["-o", "BatchMode=yes", "-o", f"IdentityAgent={agent_sock}", "-o", "IdentitiesOnly=no",
                    "-o", "PasswordAuthentication=no", "-o", "KbdInteractiveAuthentication=no"]
    return ["sshpass", "-e"], ["-o", "IdentityAgent=none", "-o", "PubkeyAuthentication=no",
                               "-o", "PreferredAuthentications=password,keyboard-interactive",
                               "-o", "NumberOfPasswordPrompts=1"]


def hop_prelude(tag: str, target_keys: str) -> str:
    """sh run on the source: a private known_hosts file holding the target's key, removed on exit.
    The file name carries the job tag so a stop can find (and kill) the hop."""
    return (
        "umask 077\n"
        f'kh=$(mktemp "${{TMPDIR:-/tmp}}/cdc-rsync-{tag}.XXXXXX") || {{ echo "cannot create a temp file on the source" >&2; exit 97; }}\n'
        "trap 'rm -f \"$kh\"' EXIT HUP INT TERM\n"
        f"printf '%s' {shlex.quote(target_keys)} > \"$kh\"\n"
    )


def hop_ssh(target: dict[str, Any]) -> str:
    """The ssh command the source uses to reach the target (via the forwarded agent). `$kh` is
    expanded by the source's sh; everything else is plain."""
    port = target["reach_port"] or target["port"]
    return (f"ssh -F /dev/null -p {port} -o BatchMode=yes -o StrictHostKeyChecking=yes "
            f'-o UserKnownHostsFile=$kh -o GlobalKnownHostsFile=/dev/null -o HostKeyAlias={TARGET_ALIAS} '
            f"-o UpdateHostKeys=no -o ConnectTimeout={SSH_TIMEOUT} -o ServerAliveInterval=30 "
            "-o ServerAliveCountMax=4 -o LogLevel=ERROR -o PasswordAuthentication=no")


def direct_script(req: dict[str, Any], tag: str, target_keys: str, src_rsync: dict[str, Any],
                  tgt_rsync: dict[str, Any]) -> str:
    """Script the source runs in direct mode: rsync source path → target over the forwarded agent."""
    argv = [src_rsync["path"], *rsync_flags(req, src_rsync, tgt_rsync),
            target_rsync_path(req["target"], tgt_rsync["path"]),
            req["source"]["path"], remote_dest(req["target"], via_reach=True)]
    return (hop_prelude(tag, target_keys)
            + f'RSYNC_RSH="{hop_ssh(req["target"])}"; export RSYNC_RSH\n'
            + " ".join(shlex.quote(a) for a in argv) + "\n")


def hop_probe_script(req: dict[str, Any], tag: str, target_keys: str) -> str:
    """Script the source runs to test the direct route: ssh to the target and probe it there."""
    t = req["target"]
    host = t["reach_host"] or t["host"]
    inner = "sh -c " + shlex.quote(probe_script(t["path"], t["rsync_path"]))
    return (hop_prelude(tag, target_keys)
            + f"{hop_ssh(t)} {shlex.quote(t['username'] + '@' + host)} {shlex.quote(inner)}\n")


def hop_keyscan_script(target: dict[str, Any]) -> str:
    host = target["reach_host"] or target["host"]
    port = target["reach_port"] or target["port"]
    return f"ssh-keyscan -T {SSH_TIMEOUT} -p {port} {shlex.quote(host)} 2>/dev/null\n"


# ── rsync output (unit-tested) ────────────────────────────────────────────────

PROGRESS_RE = re.compile(r"^\s*([\d,]+)\s+(\d{1,3})%\s+(\S+/s)\s+(\d+:\d{2}:\d{2})"
                         r"(?:\s+\(xfr#(\d+),\s*(?:ir|to)-chk=(\d+)/(\d+)\))?")
STATS_RE = re.compile(r"^(Number of [A-Za-z ]+?|Total [A-Za-z ]+?|Literal data|Matched data|File list size):\s+([\d,.]+)")
ITEM_RE = re.compile(r"^([<>ch.*][fdLDS][.+?a-zA-Z]{9,10}|\*deleting)\s+(.+)$")


def parse_progress(segment: str) -> dict[str, Any] | None:
    m = PROGRESS_RE.match(segment)
    if not m:
        return None
    out: dict[str, Any] = {"bytes": int(m.group(1).replace(",", "")), "percent": int(m.group(2)),
                           "rate": m.group(3), "eta": m.group(4)}
    if m.group(5):
        out.update(files_transferred=int(m.group(5)), files_to_check=int(m.group(6)),
                   files_total=int(m.group(7)))
    return out


def parse_stats(text: str) -> dict[str, int]:
    out: dict[str, int] = {}
    for line in text.splitlines():
        m = STATS_RE.match(line.strip())
        if m:
            key = re.sub(r"[^a-z]+", "_", m.group(1).lower()).strip("_")
            try:
                out[key] = int(float(m.group(2).replace(",", "")))
            except ValueError:
                continue
    return out


def parse_item(line: str) -> dict[str, str] | None:
    """One --itemize-changes line → {op: create|update|delete|attrs, kind, path}."""
    m = ITEM_RE.match(line.rstrip("\n"))
    if not m:
        return None
    code, path = m.group(1), m.group(2)
    if code == "*deleting":
        return {"op": "delete", "kind": "dir" if path.endswith("/") else "file", "path": path}
    kind = {"f": "file", "d": "dir", "L": "link"}.get(code[1], "other")
    if "+++++++" in code:
        op = "create"
    elif code[0] in "<>c":
        op = "update"
    else:
        op = "attrs"
    return {"op": op, "kind": kind, "path": path}


EXIT_MEANING = {
    1: "syntax or usage error (an option is not supported by this rsync)",
    2: "protocol incompatibility",
    3: "errors selecting input/output files or directories",
    5: "error starting the client-server protocol",
    10: "error in socket I/O",
    11: "error in file I/O",
    12: "error in the rsync protocol data stream",
    20: "stopped",
    23: "some files or attributes were not transferred",
    24: "some source files vanished during the transfer",
    25: "deletions stopped at the max_delete limit",
    30: "timeout in data send/receive",
    35: "timeout waiting for daemon connection",
    97: "could not create a temp file on the source",
    127: "rsync was not found",
    255: "the SSH connection failed",
}
PARTIAL_EXITS = (23, 24, 25)


def hint_for(log_tail: str) -> str | None:
    s = log_tail.lower()
    if "host key verification failed" in s or "host key mismatch" in s:
        return "SSH host key check failed. If the server was reinstalled, update or clear the pinned fingerprint."
    if any(x in s for x in ("permission denied (publickey", "permission denied, please try again",
                            "authentication failed", "wrong password")):
        return "SSH login was rejected; check the username and key/password for this host."
    if "too many authentication failures" in s:
        return "The server closed the login after too many keys; check that the right key is set."
    if "rsync: command not found" in s or "rsync: not found" in s or "rsync: no such file" in s:
        return "rsync is missing on the remote host; install it or set its rsync path."
    if "unknown option" in s or "unrecognized option" in s or "--info" in s and "unknown" in s:
        return "The rsync on one host is too old or is openrsync; install GNU rsync 3.x or set the rsync path."
    if "could not resolve hostname" in s or "name or service not known" in s:
        return "The host name does not resolve from where the connection starts (sidecar, or the source in direct mode)."
    if "connection refused" in s or "connection timed out" in s or "no route to host" in s or "operation timed out" in s:
        return "SSH port not reachable; check firewall, VPN and port. In direct mode the source must reach the target."
    if "no space left" in s or "disk quota exceeded" in s:
        return "A disk is full (the target, or the relay staging volume)."
    if "max-delete" in s:
        return "More files would be deleted than max_delete allows. Preview the run, then raise the limit if expected."
    if "permission denied" in s:
        return "The login lacks permission on some files; sync as a user that can read the source and write the target."
    if "vanished" in s:
        return "Files changed while syncing; run again, or sync from a snapshot."
    return None


def first_error(log_tail: str) -> str | None:
    for line in log_tail.splitlines():
        low = line.lower()
        if low.startswith("rsync error") or low.startswith("rsync: ") or "permission denied" in low or "ssh:" in low:
            return line.strip()[:500]
    return None


# ── per-job SSH context ───────────────────────────────────────────────────────

ASKPASS = """#!/bin/sh
# Answers one passphrase prompt from the environment; a second prompt means it was wrong.
[ -e "$CDC_ASKPASS_STATE" ] && exit 1
: > "$CDC_ASKPASS_STATE"
printf '%s\\n' "$CDC_KEY_PASSPHRASE"
"""


def base_env() -> dict[str, str]:
    """Child env without inherited agent/password variables."""
    drop = ("SSH_AUTH_SOCK", "SSHPASS", "RSYNC_RSH", "SSH_ASKPASS", "CDC_KEY_PASSPHRASE")
    return {k: v for k, v in os.environ.items() if k not in drop}


class SshContext:
    """Agents, known_hosts and helpers for one job (or one connection test).

    agent 'source' holds the source key; agent 'target' holds only the target key and is the one
    forwarded to the source in direct mode, so the source never sees its own or any other key.
    """

    def __init__(self, workdir: Path, tag: str) -> None:
        self.dir = workdir / ".secrets"
        self.dir.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.tag = tag
        self.known_hosts = self.dir / "known_hosts"
        self.known_hosts.touch(mode=0o600)
        self.agents: dict[str, tuple[subprocess.Popen[Any], str]] = {}
        self.target_known: str = ""

    def close(self) -> None:
        for proc, _ in self.agents.values():
            try:
                proc.terminate()
                proc.wait(timeout=5)
            except Exception:  # noqa: BLE001 - best effort
                proc.kill()
        self.agents.clear()
        shutil.rmtree(self.dir, ignore_errors=True)

    def agent_for(self, role: str, e: dict[str, Any]) -> str | None:
        if e["auth"] != "key":
            return None
        if role in self.agents:
            return self.agents[role][1]
        sock = str(self.dir / f"agent-{role}.sock")
        proc = subprocess.Popen(["ssh-agent", "-D", "-a", sock], stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, env=base_env(), start_new_session=True)
        self.agents[role] = (proc, sock)
        for _ in range(50):
            if os.path.exists(sock):
                break
            time.sleep(0.05)
        else:
            raise StepFailed(f"{role}: ssh-agent did not start")
        key_file = self.dir / f"key-{role}"
        fd = os.open(key_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            text = e["private_key"].replace("\r\n", "\n").strip() + "\n"
            fh.write(text)
        askpass = self.dir / "askpass"
        if not askpass.exists():
            askpass.write_text(ASKPASS, encoding="utf-8")
            askpass.chmod(0o700)
        env = base_env()
        env.update(SSH_AUTH_SOCK=sock, SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE="force", DISPLAY=":0",
                   CDC_KEY_PASSPHRASE=e["passphrase"], CDC_ASKPASS_STATE=str(self.dir / f"askpass-{role}.state"))
        try:
            out = subprocess.run(["ssh-add", "-q", str(key_file)], env=env, capture_output=True, text=True,
                                 timeout=15, stdin=subprocess.DEVNULL)
        except subprocess.TimeoutExpired:
            out = None
        finally:
            key_file.unlink(missing_ok=True)
        if out is None or out.returncode != 0:
            err = (out.stderr.strip() if out else "timed out")[:200]
            hint = ("The key is encrypted: set the passphrase." if not e["passphrase"]
                    else "The passphrase is wrong, or the key text is incomplete.")
            raise StepFailed(f"{role}: private key could not be loaded ({err or 'bad key'})", hint)
        return sock

    def trust(self, alias: str, keys: list[dict[str, str]]) -> None:
        with self.known_hosts.open("a", encoding="utf-8") as fh:
            fh.write(known_hosts_lines(alias, keys))

    def ssh_argv(self, role: str, e: dict[str, Any], *, forward_target: bool = False) -> tuple[list[str], dict[str, str]]:
        """argv prefix + env for `ssh <host>` from the sidecar."""
        alias = SOURCE_ALIAS if role == "source" else TARGET_ALIAS
        prefix, auth = ssh_auth(e, self.agent_for(role, e))
        env = base_env()
        if e["auth"] == "password":
            env["SSHPASS"] = e["password"]
        argv = [*prefix, SSH, *ssh_base(e["port"], str(self.known_hosts), alias), *auth]
        if forward_target:
            fwd = self.agents.get("target")
            if fwd is None:
                raise StepFailed("direct mode needs the target key in a forwarded agent")
            argv += ["-o", f"ForwardAgent={fwd[1]}"]
        else:
            argv += ["-o", "ForwardAgent=no"]
        argv += ["-o", "ClearAllForwardings=yes", f"{e['username']}@{e['host']}"]
        return argv, env

    def rsync_rsh(self, role: str, e: dict[str, Any]) -> tuple[str, dict[str, str]]:
        """RSYNC_RSH value + env for the sidecar's own rsync (relay mode)."""
        argv, env = self.ssh_argv(role, e)
        return " ".join(shlex.quote(a) for a in argv[:-1]), env

    def keyscan(self, e: dict[str, Any]) -> list[dict[str, str]]:
        try:
            out = subprocess.run(["ssh-keyscan", "-T", str(SSH_TIMEOUT), "-p", str(e["port"]), e["host"]],
                                 capture_output=True, text=True, timeout=SSH_TIMEOUT + 5, env=base_env())
        except subprocess.TimeoutExpired:
            return []
        return parse_keyscan(out.stdout)

    def run_remote(self, role: str, e: dict[str, Any], script: str, *, forward_target: bool = False,
                   timeout: int = 60) -> subprocess.CompletedProcess[str]:
        argv, env = self.ssh_argv(role, e, forward_target=forward_target)
        return subprocess.run([*argv, "sh -c " + shlex.quote(script)], env=env, capture_output=True,
                              text=True, timeout=timeout, stdin=subprocess.DEVNULL)


def ssh_error(stderr: str, rc: int) -> str:
    lines = [ln.strip() for ln in stderr.strip().splitlines() if ln.strip() and not ln.startswith("Warning: Permanently")]
    return ("\n".join(lines)[:500]) or f"exit code {rc}"


def connect_endpoint(ctx: SshContext, role: str, e: dict[str, Any]) -> dict[str, Any]:
    """Scan + trust the host key, log in, probe. Raises StepFailed with a hint."""
    label = "source" if role == "source" else "target"
    keys, fp, pinned = accept_keys(ctx.keyscan(e), e["host_key_fingerprint"], label)
    ctx.trust(SOURCE_ALIAS if role == "source" else TARGET_ALIAS, keys)
    if role == "target":
        ctx.target_known = known_hosts_lines(TARGET_ALIAS, keys)
    started = time.monotonic()
    try:
        out = ctx.run_remote(role, e, probe_script(e["path"], e["rsync_path"]), timeout=SSH_TIMEOUT + 30)
    except subprocess.TimeoutExpired:
        raise StepFailed(f"{label}: no answer within {SSH_TIMEOUT + 30}s",
                         "The host accepted the connection but did not respond; check sshd and the login shell.") from None
    if out.returncode != 0 or "probe=ok" not in out.stdout:
        err = ssh_error(out.stderr, out.returncode)
        if e["auth"] == "password" and out.returncode == 5:  # sshpass: password rejected
            err = "Permission denied (wrong password)"
        raise StepFailed(f"{label}: {err}", hint_for(err))
    probe = parse_probe(out.stdout)
    probe.update(latency_ms=int((time.monotonic() - started) * 1000), host_key_fingerprint=fp, host_key_verified=pinned)
    return probe


def check_route(ctx: SshContext, req: dict[str, Any]) -> dict[str, Any]:
    """Can the source log in to the target with the forwarded key? Returns the target probe."""
    t = req["target"]
    if t["auth"] != "key":
        raise StepFailed("the target uses a password; direct mode forwards a key",
                         "Use key auth on the target for direct mode, or keep relay.")
    ctx.agent_for("target", t)
    if not ctx.target_known:
        scanned = ctx.keyscan(t) if not t["reach_host"] else []
        if not scanned:  # the sidecar may not reach the target at all: ask the source
            out = ctx.run_remote("source", req["source"], hop_keyscan_script(t), timeout=SSH_TIMEOUT + 15)
            scanned = parse_keyscan(out.stdout)
        keys, fp, pinned = accept_keys(scanned, t["host_key_fingerprint"], "target (from source)")
        ctx.target_known = known_hosts_lines(TARGET_ALIAS, keys)
        ctx.target_fp = (fp, pinned)  # type: ignore[attr-defined]
    try:
        out = ctx.run_remote("source", req["source"], hop_probe_script(req, ctx.tag, ctx.target_known),
                             forward_target=True, timeout=2 * SSH_TIMEOUT + 30)
    except subprocess.TimeoutExpired:
        raise StepFailed("source → target: no answer", "The source could not finish an SSH login to the target in time.") from None
    if out.returncode != 0 or "probe=ok" not in out.stdout:
        err = ssh_error(out.stderr, out.returncode)
        hint = hint_for(err)
        if "permission denied" in err.lower():
            hint = ("The source reached the target but the forwarded key was refused. Check the target key, and "
                    "that the source's sshd allows agent forwarding (AllowAgentForwarding yes).")
        if out.returncode == 127 or "ssh: not found" in err or "ssh: command not found" in err:
            hint = "The source has no ssh client."
        raise StepFailed(f"source → target: {err}", hint)
    return parse_probe(out.stdout)


# ── job store ─────────────────────────────────────────────────────────────────


def job_dir(job_id: str) -> Path:
    if not JOB_ID.match(job_id):
        raise BadRequest("invalid job id", 404)
    return JOBS_DIR / job_id


def read_json(path: Path) -> dict[str, Any]:
    if not path.is_file():
        return {}
    with path.open("r", encoding="utf-8") as fh:
        return json.load(fh)


def write_json(path: Path, data: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    with tmp.open("w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2)
    tmp.replace(path)


def update_meta(job_id: str, **fields: Any) -> dict[str, Any]:
    with _lock:
        path = job_dir(job_id) / "job.json"
        meta = read_json(path)
        meta.update(fields)
        write_json(path, meta)
        return meta


def set_rt(job_id: str, **fields: Any) -> None:
    with _lock:
        if job_id in _runtime:
            _runtime[job_id].update(fields)


def job_status(job_id: str) -> dict[str, Any]:
    meta = read_json(job_dir(job_id) / "job.json")
    if not meta:
        return {"id": job_id, "status": "unknown"}
    with _lock:
        rt = dict(_runtime.get(job_id) or {})
    meta.update(rt)
    meta["id"] = job_id
    return meta


def tail_log(path: Path, max_bytes: int = 256_000) -> str:
    if not path.is_file():
        return ""
    data = path.read_bytes()
    if len(data) > max_bytes:
        data = data[-max_bytes:]
    return data.decode("utf-8", errors="replace")


def active_jobs() -> list[str]:
    with _lock:
        return [j for j, rt in _runtime.items() if rt.get("status") in ACTIVE]


def free_mb() -> int:
    try:
        return shutil.disk_usage(WORK_ROOT).free // (1024 * 1024)
    except OSError:
        return -1


def staging_path(req: dict[str, Any], job_id: str) -> Path:
    return STAGING_DIR / (req["profile_id"] or f"job-{job_id}")


# ── execution ─────────────────────────────────────────────────────────────────

MAX_CHANGES = 500


def _run_step(job_id: str, phase: str, argv: list[str], env: dict[str, str], log_fh: Any,
              deadline: float | None, show: list[str] | None = None) -> tuple[int, str]:
    """Run one rsync (local, or ssh → source). Progress lines (\\r) update the job; other lines go
    to the log. Returns (exit code, this step's output for stats/item parsing)."""
    shown = " ".join(shlex.quote(a) for a in (show or argv))
    log_fh.write(f"\n--- {phase} at {utc_now()} ---\n{shown}\n".encode())
    log_fh.flush()
    with _lock:
        if job_id in _stop_requested:
            return -signal.SIGTERM, ""
        proc = subprocess.Popen(argv, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                stdin=subprocess.DEVNULL, cwd=str(job_dir(job_id)), start_new_session=True)
        _procs[job_id] = proc
        _runtime[job_id].update({"phase": phase, "progress": None})
    captured: list[str] = []
    counts = {"create": 0, "update": 0, "delete": 0, "attrs": 0}
    changes: list[dict[str, str]] = []

    def handle(segment: str, newline: bool) -> None:
        prog = parse_progress(segment)
        if prog is not None:
            set_rt(job_id, progress=prog)
            return
        if not segment.strip():
            return
        if newline:
            log_fh.write((segment + "\n").encode())
            captured.append(segment)
            item = parse_item(segment)
            if item is not None:
                counts[item["op"]] += 1
                if len(changes) < MAX_CHANGES:
                    changes.append(item)
        set_rt(job_id, last_message=segment.strip()[:300])

    def pump() -> None:
        assert proc.stdout is not None
        buf = ""
        while True:
            chunk = proc.stdout.read1(65536) if hasattr(proc.stdout, "read1") else proc.stdout.read(4096)
            if not chunk:
                break
            buf += chunk.decode("utf-8", errors="replace")
            while True:
                m = re.search(r"[\r\n]", buf)
                if not m:
                    break
                handle(buf[:m.start()], m.group() == "\n")
                buf = buf[m.end():]
        if buf:
            handle(buf, True)
        log_fh.flush()
        with _lock:
            rt = _runtime.get(job_id)
            if rt is not None and any(counts.values()):
                rt["changes"] = {"counts": counts, "items": changes, "truncated": sum(counts.values()) > len(changes)}

    t = threading.Thread(target=pump, daemon=True)
    t.start()
    while True:
        try:
            code = proc.wait(timeout=1)
            break
        except subprocess.TimeoutExpired:
            if deadline and time.monotonic() > deadline:
                _killpg(proc)
                t.join(5)
                raise TimeoutError(f"{phase} exceeded RSYNC_TIMEOUT_SECONDS={TIMEOUT_SECONDS}")
    t.join(10)
    if proc.stdout is not None:
        proc.stdout.close()
    with _lock:
        _procs.pop(job_id, None)
    return code, "\n".join(captured)


def _killpg(proc: subprocess.Popen[Any]) -> None:
    try:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def decide_transport(ctx: SshContext, job_id: str, req: dict[str, Any], log_fh: Any) -> tuple[str, str, dict[str, Any] | None]:
    """(transport, reason, target probe via source). auto falls back to relay with the reason."""
    want = req["transport"]
    if want == "relay":
        return "relay", "relay chosen in the profile", None
    try:
        set_rt(job_id, phase="route")
        probe = check_route(ctx, req)
        log_fh.write(f"direct route ok: source reached {req['target']['reach_host'] or req['target']['host']}\n".encode())
        return "direct", "the source reaches the target", probe
    except StepFailed as exc:
        if want == "direct":
            raise
        log_fh.write(f"direct route unavailable ({exc}); falling back to relay\n".encode())
        return "relay", f"direct route unavailable: {exc}", None


def outcome(code: int, stopped: bool) -> tuple[str, str | None]:
    if stopped:
        return "stopped", None
    if code == 0:
        return "succeeded", None
    meaning = EXIT_MEANING.get(code, "failed")
    if code in PARTIAL_EXITS:
        return "completed_with_errors", f"rsync exit {code}: {meaning}"
    return "failed", f"rsync exit {code}: {meaning}"


def run_job(job_id: str, req: dict[str, Any]) -> None:
    work = job_dir(job_id)
    log_path = work / "rsync.log"
    deadline = time.monotonic() + TIMEOUT_SECONDS if TIMEOUT_SECONDS > 0 else None
    status, error, hint, exit_code = "failed", None, None, None
    fields: dict[str, Any] = {}
    ctx = SshContext(work, job_id[:8])
    staging: Path | None = None
    try:
        update_meta(job_id, status="running", started_at=utc_now())
        set_rt(job_id, status="running", phase="connect")
        with log_path.open("ab") as log_fh:
            src, tgt = req["source"], req["target"]
            log_fh.write(f"--- connect at {utc_now()} ---\n".encode())
            src_probe = connect_endpoint(ctx, "source", src)
            fields["host_keys"] = {"source": {"fingerprint": src_probe["host_key_fingerprint"],
                                              "verified": src_probe["host_key_verified"]}}
            if src_probe.get("kind") == "missing":
                raise StepFailed(f"source path {src['path']!r} does not exist on {src['host']}",
                                 "Check the path; relative paths start in the login's home directory.")
            if src_probe["rsync"] is None:
                raise StepFailed(f"rsync not found on the source {src['host']}", rsync_hint(src_probe))
            log_fh.write(f"source: {src_probe.get('os')} {src_probe.get('arch')}, {src_probe['rsync']['path']} "
                         f"({src_probe['rsync']['version']})\n".encode())

            transport, reason, tgt_probe = decide_transport(ctx, job_id, req, log_fh)
            if transport == "relay":
                set_rt(job_id, phase="connect")
                tgt_probe = connect_endpoint(ctx, "target", tgt)
                fields["host_keys"]["target"] = {"fingerprint": tgt_probe["host_key_fingerprint"],
                                                 "verified": tgt_probe["host_key_verified"]}
            else:
                fp, pinned = getattr(ctx, "target_fp", (None, False))
                if fp:
                    fields["host_keys"]["target"] = {"fingerprint": fp, "verified": pinned, "seen_from": "source"}
            assert tgt_probe is not None
            if tgt_probe["rsync"] is None:
                raise StepFailed(f"rsync not found on the target {tgt['host']}", rsync_hint(tgt_probe))
            fields.update(transport=transport, transport_reason=reason,
                          hosts={"source": _host_facts(src_probe), "target": _host_facts(tgt_probe)})
            update_meta(job_id, **fields)
            set_rt(job_id, transport=transport, transport_reason=reason)
            log_fh.write(f"target: {tgt_probe.get('os')} {tgt_probe.get('arch')}, {tgt_probe['rsync']['path']} "
                         f"({tgt_probe['rsync']['version']})\ntransport: {transport} ({reason})\n".encode())

            if transport == "direct":
                check_compat(req, {"source": src_probe["rsync"], "target": tgt_probe["rsync"]})
                script = direct_script(req, ctx.tag, ctx.target_known, src_probe["rsync"], tgt_probe["rsync"])
                argv, env = ctx.ssh_argv("source", src, forward_target=True)
                with _lock:
                    _stoppers[job_id] = (ctx, src)
                exit_code, out = _run_step(job_id, "sync", [*argv, "sh -c " + shlex.quote(script)], env, log_fh,
                                           deadline, show=[*argv[:1], "…", f"{src['username']}@{src['host']}",
                                                           "sh -c <direct script>"])
                fields["stats"] = parse_stats(out)
            else:
                local = rsync_flavor(subprocess.check_output([RSYNC, "--version"], text=True).splitlines()[0])
                check_compat(req, {"source": src_probe["rsync"], "target": tgt_probe["rsync"], "sidecar": local})
                staging = staging_path(req, job_id)
                staging.mkdir(parents=True, exist_ok=True)
                if MIN_FREE_MB > 0 and 0 <= free_mb() < MIN_FREE_MB:
                    raise StepFailed(f"relay staging volume has {free_mb()} MB free; need {MIN_FREE_MB} MB",
                                     "Free space on the cdc-rsync volume or raise RSYNC_MIN_FREE_MB.")
                rsh, env = ctx.rsync_rsh("source", src)
                env["RSYNC_RSH"] = rsh
                # --fake-super: staging is written by a non-root user, so owners/modes live in xattrs
                # there and are restored on the push (a root target login then keeps real owners).
                pull = [RSYNC, *rsync_flags(req, local, src_probe["rsync"], delete=True, dry_run=False, mirror=True),
                        "--fake-super", f"--rsync-path={src_probe['rsync']['path']}",
                        f"{src['username']}@{host_spec(src['host'])}:{src['path']}", f"{staging}/"]
                pull_code, out = _run_step(job_id, "pull", pull, env, log_fh, deadline)
                exit_code = pull_code
                fields["stats_pull"] = parse_stats(out)
                with _lock:
                    _runtime[job_id].pop("changes", None)  # only the push decides what the target sees
                # 23/24: some source files were unreadable or vanished. rsync skips deletions after
                # I/O errors, so staging still holds the last good copy and pushing is safe.
                if pull_code in (0, 23, 24) and job_id not in _stop_requested:
                    rsh, env = ctx.rsync_rsh("target", tgt)
                    env["RSYNC_RSH"] = rsh
                    push = [RSYNC, *rsync_flags(req, local, tgt_probe["rsync"]), "--fake-super",
                            target_rsync_path(tgt, tgt_probe["rsync"]["path"]), f"{staging}/", remote_dest(tgt)]
                    push_code, out = _run_step(job_id, "push", push, env, log_fh, deadline)
                    fields["stats"] = parse_stats(out)
                    exit_code = push_code or pull_code
            status, error = outcome(exit_code, job_id in _stop_requested)
            if req["flags"]["dry_run"] and status == "succeeded":
                fields["dry_run"] = True
    except StepFailed as exc:
        error, hint = str(exc), exc.hint
    except TimeoutError as exc:
        error = str(exc)
    except Exception as exc:  # noqa: BLE001 - surface any failure on the job
        error = f"{type(exc).__name__}: {exc}"
    finally:
        ctx.close()
        with _lock:
            rt = _runtime.pop(job_id, {})
            stopped = job_id in _stop_requested
            _stop_requested.discard(job_id)
            _procs.pop(job_id, None)
            _stoppers.pop(job_id, None)
        if stopped and status == "failed":
            status, error, hint = "stopped", None, None
        if staging is not None and req["flags"]["discard_staging"]:
            shutil.rmtree(staging, ignore_errors=True)
            fields["staging_discarded"] = True
        tail = tail_log(log_path, 16000)
        fields.update(status=status, finished_at=utc_now(), exit_code=exit_code)
        for key in ("changes", "progress", "transport", "transport_reason", "phase"):
            if key in rt and rt[key] is not None:
                fields[key] = rt[key]
        if error:
            fields["error"] = error
            fe = first_error(tail)
            if fe and fe not in error:
                fields["first_error"] = fe
            hint = hint or hint_for(tail if exit_code else error)
            if hint:
                fields["hint"] = hint
        update_meta(job_id, **fields)


def _host_facts(probe: dict[str, Any]) -> dict[str, Any]:
    r = probe.get("rsync") or {}
    return {"os": probe.get("os"), "arch": probe.get("arch"), "rsync_path": r.get("path"),
            "rsync_version": r.get("version"), "gnu_rsync": r.get("gnu"), "path_kind": probe.get("kind"),
            "free_bytes": probe.get("free_bytes")}


def start_job(body: dict[str, Any]) -> dict[str, Any]:
    req = validate_request(body)
    with _lock:
        running = [rt for rt in _runtime.values() if rt.get("status") in ACTIVE]
        if req["profile_id"] and any(rt.get("profile_id") == req["profile_id"] for rt in running):
            raise BadRequest("a sync for this profile is already running", 409)
        if len(running) >= MAX_CONCURRENT:
            raise BadRequest(f"{len(running)} sync job(s) already running (RSYNC_MAX_CONCURRENT)", 429)
        job_id = str(uuid.uuid4())
        _runtime[job_id] = {"status": "starting", "profile_id": req["profile_id"]}
    work = job_dir(job_id)
    work.mkdir(parents=True, exist_ok=True)
    meta = {
        "id": job_id,
        "name": req["name"] or job_id,
        "profile_id": req["profile_id"],
        "status": "starting",
        "created_at": utc_now(),
        "source": describe_endpoint(req["source"]),
        "target": describe_endpoint(req["target"]),
        "transport_requested": req["transport"],
        "dry_run": req["flags"]["dry_run"],
        "delete": req["flags"]["delete"],
        "excludes": req["excludes"],
    }
    write_json(work / "job.json", meta)
    threading.Thread(target=run_job, args=(job_id, req), daemon=True, name=f"rsync-{job_id[:8]}").start()
    return job_status(job_id)


def stop_job(job_id: str) -> dict[str, Any]:
    with _lock:
        proc = _procs.get(job_id)
        if job_id in _runtime:
            _stop_requested.add(job_id)
        stopper = _stoppers.get(job_id)
    if stopper is not None:
        # Direct mode: the rsync runs on the source. Its hop ssh carries the job tag in the
        # known_hosts path, so kill it there; the source-side rsync then exits too.
        ctx, src = stopper
        try:
            ctx.run_remote("source", src, f"pkill -f {shlex.quote('cdc-rsync-' + ctx.tag)} || true", timeout=20)
        except Exception:  # noqa: BLE001 - best effort; the local kill below still ends the session
            pass
    if proc is not None and proc.poll() is None:
        _killpg(proc)
    return job_status(job_id)


def delete_job(job_id: str) -> bool:
    work = job_dir(job_id)
    if not work.is_dir():
        return False
    if job_id in active_jobs():
        raise BadRequest("job is running; stop it first", 409)
    shutil.rmtree(work, ignore_errors=True)
    return True


def list_jobs(profile_id: str | None = None) -> list[dict[str, Any]]:
    sweep_expired()
    if not JOBS_DIR.is_dir():
        return []
    jobs = [job_status(c.name) for c in JOBS_DIR.iterdir() if c.is_dir() and JOB_ID.match(c.name)]
    if profile_id:
        jobs = [j for j in jobs if j.get("profile_id") == profile_id]
    jobs.sort(key=lambda j: j.get("created_at") or "", reverse=True)
    return jobs


def sweep_expired() -> None:
    busy = set(active_jobs())
    if RETENTION_HOURS > 0 and JOBS_DIR.is_dir():
        cutoff = datetime.now(timezone.utc) - timedelta(hours=RETENTION_HOURS)
        for child in JOBS_DIR.iterdir():
            if not child.is_dir() or child.name in busy:
                continue
            finished = read_json(child / "job.json").get("finished_at")
            try:
                if finished and datetime.fromisoformat(finished) < cutoff:
                    shutil.rmtree(child, ignore_errors=True)
            except ValueError:
                continue
    if STAGING_RETENTION_DAYS > 0 and STAGING_DIR.is_dir():
        cutoff_ts = time.time() - STAGING_RETENTION_DAYS * 86400
        busy_profiles = {(_runtime.get(j) or {}).get("profile_id") for j in busy}
        for child in STAGING_DIR.iterdir():
            if child.is_dir() and child.name not in busy_profiles and child.stat().st_mtime < cutoff_ts:
                shutil.rmtree(child, ignore_errors=True)


def staging_list() -> list[dict[str, Any]]:
    if not STAGING_DIR.is_dir():
        return []
    out = []
    for child in sorted(STAGING_DIR.iterdir()):
        if not child.is_dir():
            continue
        size = 0
        for root, _dirs, files in os.walk(child):
            for f in files:
                try:
                    size += os.lstat(os.path.join(root, f)).st_size
                except OSError:
                    pass
        out.append({"id": child.name, "bytes": size,
                    "modified_at": datetime.fromtimestamp(child.stat().st_mtime, timezone.utc).isoformat()})
    return out


def staging_delete(staging_id: str) -> bool:
    if not JOB_ID.match(staging_id):
        raise BadRequest("invalid staging id", 404)
    with _lock:
        if any(rt.get("profile_id") == staging_id and rt.get("status") in ACTIVE for rt in _runtime.values()):
            raise BadRequest("a sync for this profile is running", 409)
    path = STAGING_DIR / staging_id
    if not path.is_dir():
        return False
    shutil.rmtree(path, ignore_errors=True)
    return True


def recover_on_start() -> None:
    """A restart orphans running jobs: mark them failed and remove leftover secrets."""
    if not JOBS_DIR.is_dir():
        return
    for child in JOBS_DIR.iterdir():
        if not child.is_dir():
            continue
        shutil.rmtree(child / ".secrets", ignore_errors=True)
        meta = read_json(child / "job.json")
        if meta.get("status") in ACTIVE:
            meta.update(status="failed", error="sidecar restarted while the job was running", finished_at=utc_now())
            write_json(child / "job.json", meta)


def tool_versions() -> dict[str, str]:
    out = {}
    for name, argv in (("rsync", [RSYNC, "--version"]), ("ssh", [SSH, "-V"])):
        try:
            res = subprocess.run(argv, capture_output=True, text=True, timeout=5)
            out[name] = (res.stdout or res.stderr).strip().splitlines()[0]
        except (OSError, subprocess.SubprocessError, IndexError):
            out[name] = "unavailable"
    return out


# ── connection tests ──────────────────────────────────────────────────────────


def _probe_answer(e: dict[str, Any], side: str, probe: dict[str, Any]) -> dict[str, Any]:
    out = {"ok": True, "target": describe_endpoint(e), "side": side, **_host_facts(probe),
           "latency_ms": probe.get("latency_ms"), "host_key_fingerprint": probe.get("host_key_fingerprint"),
           "host_key_verified": probe.get("host_key_verified"), "writable": probe.get("writable"),
           "ssh_client": probe.get("ssh")}
    warnings = []
    hint = rsync_hint(probe)
    if probe.get("rsync") is None:
        return {**out, "ok": False, "error": "rsync not found", "hint": hint}
    if hint:
        warnings.append(hint)
    if side == "source" and probe.get("kind") == "missing":
        return {**out, "ok": False, "error": f"path {e['path']!r} does not exist", "hint": "Relative paths start in the login's home."}
    if side == "target" and probe.get("kind") == "missing" and not probe.get("writable"):
        warnings.append("The path does not exist and its parent is not writable for this login.")
    if side == "target" and probe.get("kind") == "file":
        return {**out, "ok": False, "error": "the target path is a file; it must be a directory"}
    if side == "source" and not probe.get("ssh"):
        warnings.append("No ssh client on the source: direct mode is not possible, relay will be used.")
    if warnings:
        out["warning"] = " ".join(warnings)
    return out


def test_connection(body: dict[str, Any]) -> dict[str, Any]:
    """Log in from this container (where relay runs and where direct starts) and describe the host.
    A failure is a normal answer (ok=false, error, hint)."""
    side = str(body.get("side") or "source")
    if side not in ("source", "target"):
        raise BadRequest("side must be source or target")
    e = norm_endpoint(body.get("endpoint"), side)
    WORK_ROOT.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(prefix=".test-", dir=WORK_ROOT))
    ctx = SshContext(tmp, uuid.uuid4().hex[:8])
    try:
        return _probe_answer(e, side, connect_endpoint(ctx, side, e))
    except StepFailed as exc:
        return {"ok": False, "target": describe_endpoint(e), "side": side, "error": str(exc), "hint": exc.hint}
    except subprocess.TimeoutExpired:
        return {"ok": False, "target": describe_endpoint(e), "side": side, "error": "timed out",
                "hint": "The host did not answer in time."}
    finally:
        ctx.close()
        shutil.rmtree(tmp, ignore_errors=True)


def test_route(body: dict[str, Any]) -> dict[str, Any]:
    """Would a run use direct mode? Logs in to the source and, from there, to the target."""
    req = validate_request({**body, "options": {"transport": "auto"}})
    tmp = Path(tempfile.mkdtemp(prefix=".route-", dir=WORK_ROOT))
    ctx = SshContext(tmp, uuid.uuid4().hex[:8])
    try:
        connect_endpoint(ctx, "source", req["source"])
        probe = check_route(ctx, req)
        fp, pinned = getattr(ctx, "target_fp", (None, False))
        return {"direct": True, "transport": "direct", "target": _host_facts(probe),
                "target_host_key_fingerprint": fp, "target_host_key_verified": pinned,
                "warning": rsync_hint(probe)}
    except StepFailed as exc:
        return {"direct": False, "transport": "relay", "error": str(exc), "hint": exc.hint}
    finally:
        ctx.close()
        shutil.rmtree(tmp, ignore_errors=True)


CAPABILITY = {
    "service": "cdc-rsync",
    "transports": list(TRANSPORTS),
    "options": {
        "booleans": sorted(list(BOOL_FLAGS) + list(OTHER_BOOLS)),
        "integers": sorted(INT_OPTIONS),
        "lists": ["excludes"],
        "strings": ["transport"],
    },
    "endpoint_fields": ["name", "host", "port", "username", "auth (key|password)", "password", "private_key",
                        "passphrase", "path", "host_key_fingerprint (SHA256:...)", "rsync_path",
                        "reach_host / reach_port (target: address the source uses)"],
    "behaviour": {
        "direct": "ssh -A to the source; rsync there pushes to the target with the forwarded key",
        "relay": "pull source → staging volume (mirror), then push staging → target",
        "auto": "direct when the source can log in to the target, else relay",
        "target_path": "always a directory; created if missing",
    },
}


# ── HTTP ──────────────────────────────────────────────────────────────────────


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt: str, *args: Any) -> None:
        return

    def _auth_ok(self) -> bool:
        if not API_TOKEN:
            return True
        auth = self.headers.get("Authorization", "")
        if auth.startswith("Bearer ") and auth[7:].strip() == API_TOKEN:
            return True
        return self.headers.get("X-API-Token", "").strip() == API_TOKEN

    def _json(self, code: int, payload: Any) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _read_body(self) -> dict[str, Any]:
        length = int(self.headers.get("Content-Length", "0") or "0")
        raw = self.rfile.read(length) if length else b"{}"
        try:
            data = json.loads(raw.decode("utf-8"))
        except json.JSONDecodeError:
            raise BadRequest("body must be JSON") from None
        if not isinstance(data, dict):
            raise BadRequest("body must be a JSON object")
        return data

    def _route(self, method: str) -> None:
        if not self._auth_ok():
            self._json(401, {"error": "unauthorized"})
            return
        url = urlparse(self.path)
        parts = [p for p in url.path.split("/") if p]
        try:
            self._dispatch(method, parts, parse_qs(url.query))
        except BadRequest as exc:
            self._json(exc.code, {"error": str(exc)})
        except BrokenPipeError:
            pass
        except Exception as exc:  # noqa: BLE001
            self._json(500, {"error": f"{type(exc).__name__}: {exc}"})

    def _dispatch(self, method: str, parts: list[str], query: dict[str, list[str]]) -> None:
        if method == "GET" and parts == ["health"]:
            self._json(200, {"status": "ok"})
        elif method == "GET" and parts in (["v1", "discover"], ["v1", "capability"]):
            out = dict(CAPABILITY)
            out.update(tool_versions())
            out.update({"free_mb": free_mb(), "max_concurrent": MAX_CONCURRENT, "retention_hours": RETENTION_HOURS,
                        "staging_retention_days": STAGING_RETENTION_DAYS, "timeout_seconds": TIMEOUT_SECONDS})
            if parts[1] == "discover":
                out.update({"contract_version": "1", "tool_id": "rsync", "sidecar_ready": True,
                            "orchestrator_contract": "/cdc/rsync/contract"})
            self._json(200, out)
        elif parts == ["v1", "jobs"] and method == "GET":
            self._json(200, {"jobs": list_jobs((query.get("profile_id") or [None])[0])})
        elif parts == ["v1", "jobs"] and method == "POST":
            self._json(202, start_job(self._read_body()))
        elif parts == ["v1", "test-connection"] and method == "POST":
            self._json(200, test_connection(self._read_body()))
        elif parts == ["v1", "test-route"] and method == "POST":
            self._json(200, test_route(self._read_body()))
        elif parts == ["v1", "staging"] and method == "GET":
            self._json(200, {"staging": staging_list(), "free_mb": free_mb()})
        elif len(parts) == 3 and parts[:2] == ["v1", "staging"] and method == "DELETE":
            if staging_delete(parts[2]):
                self._json(200, {"id": parts[2], "deleted": True})
            else:
                self._json(404, {"error": "no staging for this id"})
        elif len(parts) == 3 and parts[:2] == ["v1", "jobs"]:
            job_id = parts[2]
            if method == "GET":
                meta = job_status(job_id)
                self._json(404 if meta["status"] == "unknown" else 200,
                           {"error": "job not found"} if meta["status"] == "unknown" else meta)
            elif method == "DELETE":
                if delete_job(job_id):
                    self._json(200, {"id": job_id, "deleted": True})
                else:
                    self._json(404, {"error": "job not found"})
            else:
                self._json(405, {"error": "method not allowed"})
        elif len(parts) == 4 and parts[:2] == ["v1", "jobs"]:
            job_id, action = parts[2], parts[3]
            if action == "logs" and method == "GET":
                self._json(200, {"id": job_id, "log": tail_log(job_dir(job_id) / "rsync.log")})
            elif action == "stop" and method == "POST":
                meta = job_status(job_id)
                if meta["status"] == "unknown":
                    self._json(404, {"error": "job not found"})
                else:
                    self._json(200, stop_job(job_id))
            else:
                self._json(404, {"error": "not found"})
        else:
            self._json(404, {"error": "not found"})

    def do_GET(self) -> None:
        self._route("GET")

    def do_POST(self) -> None:
        self._route("POST")

    def do_DELETE(self) -> None:
        self._route("DELETE")


def main() -> None:
    JOBS_DIR.mkdir(parents=True, exist_ok=True)
    STAGING_DIR.mkdir(parents=True, exist_ok=True)
    recover_on_start()
    sweep_expired()
    server = ThreadingHTTPServer((HOST, PORT), Handler)
    print(f"cdc-rsync listening on {HOST}:{PORT} (work_root={WORK_ROOT})", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
