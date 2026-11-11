# ADR 0012 — File sync: rsync daemon (`rsync://`) endpoints, meant for WireGuard tunnels

Status: Accepted — 2026-11-07

## Context

File sync (ADR 0007) logs in to each host over SSH. Some sites run `rsync --daemon` (NAS boxes,
backup servers, hosts where no SSH login may be issued) and, since ADR 0010, can be reached only
through a WireGuard tunnel. ADR 0007 rejected the daemon because its protocol is plain text and its
auth is weaker than SSH; inside a tunnel the tunnel supplies both encryption and the network
boundary, so that objection mostly falls away.

## Decision

- An endpoint has `protocol`: `ssh` (default, unchanged) or `daemon`. A daemon endpoint is
  `rsync://[user@]host:port/module/path`: host, port (873), **module** (an rsyncd.conf `[name]`),
  optional path inside the module, optional daemon user + password (`auth users` / secrets file;
  both empty = anonymous module).
- The password is passed to rsync only in `RSYNC_PASSWORD`, never on the command line; it is stored
  and redacted like SSH passwords (reused only for the same host/port/user).
- Daemon endpoints **always use the relay** (pull to the sidecar's staging, push from it): there is
  no shell on a daemon host, so no direct host-to-host copy. `transport=direct` with a daemon side is
  rejected. Each side may be SSH or daemon independently.
- Test connection lists the module (`rsync --list-only`): reachability, module, credentials and
  whether the path exists. Writing to a target module is first exercised by the run (`read only = no`).
- Daemon `@ERROR` answers (auth failed, unknown module, access denied, read only, max connections)
  map to specific hints.
- Through a tunnel, a daemon endpoint's forward defaults to port 873. Without a tunnel, the editor
  warns that the protocol is not encrypted.
- Tunnel readiness offers **rsync daemon (File sync)**: an `rsyncd.conf` bound to the site's tunnel
  address, `hosts allow` = the gateway's tunnel IP, secrets file 600, and the File sync check that
  confirms the module and login.

## Consequences

- No host key, OS or rsync-version facts for a daemon side (shown as "rsync daemon"); owners are kept
  only when the module runs as root (`uid = root`) — otherwise files take the module user.
- The target's parent folders must exist in the module (the daemon creates only the last one).
- Code: `docker/rsync/serve.py` (`norm_daemon_endpoint`, `connect_daemon`, `daemon_url`),
  `src/rsync.rs` (`check_daemon_endpoint`), `cdc-native-ui/components/rsync/RsyncEndpoint.vue`,
  `utils/tunnelReadiness.ts` (`rsyncd`). Live test: `DaemonLiveTest` in `docker/rsync/test_serve.py`
  (needs GNU rsync; run in the sidecar image).
