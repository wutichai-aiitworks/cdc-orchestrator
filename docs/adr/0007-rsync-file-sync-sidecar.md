# ADR 0007 — Host-to-host file sync runs in a `cdc-rsync` sidecar, direct first, relay as fallback

Status: Accepted
Owner: TBD
Last updated: 2026-11-03

## Context

Operators need to copy directories between servers: Ubuntu to Ubuntu, other Linux, and macOS
(a Mac mini as backup target, or a Mac as source). rsync over SSH is the standard tool, but rsync
needs one side to be local, and the orchestrator is a third machine. Two shapes are possible:

- **direct** — log in to the source and run rsync there, pushing to the target. Data crosses the
  network once and rsync's delta transfer works end to end. Needs the source to reach the target
  and to authenticate to it.
- **relay** — pull source → sidecar, then push sidecar → target. Works when the hosts cannot see
  each other (different VPNs, DMZ) but moves data twice and needs disk on the sidecar.

macOS adds a twist: since macOS 15 `/usr/bin/rsync` is openrsync (no `--info=progress2`, `-A`, `-X`),
older macOS ships rsync 2.6.9, and non-interactive SSH sessions do not have Homebrew on `PATH`.

## Decision

1. **Sidecar with the pgdump shape, not `sidecar_common`.** `docker/rsync/` (stdlib Python
   `serve.py` on Debian bookworm with rsync, OpenSSH 9.2, sshpass) behind `/cdc/rsync/*`; profiles in
   rqlite (kind `rsync_profile`). A run is several steps (connect, route check, sync, or pull + push)
   with per-job agents, so it keeps pgdump's own runner instead of ADR 0006's single-process job store.
   ADR 0006's guarantees still hold: a new job id per run, one active run per profile, secrets removed
   when the run ends, connection tests run in the sidecar.
2. **`transport = auto | direct | relay`, default `auto`.** Each auto run first checks the direct route
   (sidecar → source → target login and probe); if it fails, the run uses relay and records
   `transport_reason`. `direct` fails instead of falling back. The choice is per run, so a network
   change does not need a profile edit.
3. **Direct mode lends the target key, never stores it.** The sidecar starts two `ssh-agent`s per
   job: one holds the source key (sidecar → source login), the other only the target key and is
   forwarded with `ForwardAgent=<socket>`. The source can use the target key for the run but cannot
   read it, and the source key is never forwarded. Consequently a **password target is never direct**
   (the password would have to reach the source); `auto` relays it, `direct` is rejected at save time.
4. **Host keys are checked on every hop.** The sidecar scans each host key (`ssh-keyscan`), checks it
   against the pinned `SHA256:` fingerprint when set, and trusts only the matching key
   (`StrictHostKeyChecking=yes`, a per-job `known_hosts`, `HostKeyAlias`). For the source → target hop
   the target's key line is written to a `mktemp` file on the source, removed on exit, and used with
   `HostKeyAlias=cdc-rsync-target`, so the source may reach the target under another address
   (`reach_host` / `reach_port`). If the sidecar cannot reach the target at all, the key is scanned
   from the source. Unpinned runs report the fingerprint for one-click pinning.
5. **Secrets never on argv, in `job.json` or logs.** Keys go into the agents through a `0600` file
   deleted right after `ssh-add`; passphrases reach `ssh-add` through an `SSH_ASKPASS` script that
   reads the environment once (a second prompt means a wrong passphrase, not a retry loop).
   Passwords reach `sshpass -e` through `SSHPASS`. The job's `.secrets/` directory is removed at the
   end and on sidecar start. Saved secrets follow ADR 0005 §11 for password, private key and passphrase.
6. **rsync is detected, not assumed.** A POSIX `sh` probe on each host lists `/opt/homebrew/bin/rsync`,
   `/usr/local/bin/rsync`, `/usr/bin/rsync`, `rsync` and picks GNU ≥ 3.1, then any GNU, then whatever
   exists; `rsync_path` overrides. Flags are chosen per pair: `--info=progress2` only when the client
   has it, `--protect-args` only when both sides do (otherwise paths with spaces or shell characters
   are refused), and `acls`/`xattrs` fail early unless both sides are GNU.
7. **Path rules are rsync's, made explicit.** The source path is used verbatim (trailing `/` = copy the
   contents; the UI says so). The target path is always a directory, created with
   `--rsync-path="mkdir -p <dir> && <rsync>"`, so a single file never lands as a file named like the
   directory. Relative paths start in the login's home.
8. **Deletion is opt-in and guarded.** `delete` is off by default; `max_delete` maps to
   `--max-delete` (exit 25 → `completed_with_errors` with a hint). Runs with `delete` are confirmed in
   the UI with the exact effect. **Preview** (`POST …/run?dry_run=true`, or `options.dry_run`) runs
   `--dry-run --itemize-changes` and returns counts plus the first 500 changes.
9. **Relay keeps one staging copy per profile** (`staging/<profile_id>`), mirrored from the source
   (`--delete --delete-excluded`), so relay runs are incremental on both legs. Staging is written by a
   non-root user with `--fake-super` (owners and modes kept in xattrs and restored on the push).
   `discard_staging` removes it after each run; unused copies expire after
   `RSYNC_STAGING_RETENTION_DAYS`; deleting the profile removes its copy. A pull that ends with 23/24
   still pushes (rsync skips deletions after I/O errors, so staging keeps the last good copy).
10. **Exit codes map to statuses.** 0 → `succeeded`; 23/24/25 → `completed_with_errors`;
    stop → `stopped`; anything else → `failed` with `first_error` and an operator `hint`. A stop in
    direct mode also kills the source-side rsync (`pkill -f cdc-rsync-<job>`, the job tag is in the
    hop's known_hosts path).

## Consequences

- The source needs an SSH client and `AllowAgentForwarding yes` (the OpenSSH default) for direct mode;
  otherwise runs relay, which needs disk on `cdc-rsync-data` (`RSYNC_MIN_FREE_MB`).
- Relay previews still refresh the staging copy (a real pull), because the push diff needs it.
- Job state is per sidecar instance; a sidecar restart marks running jobs failed.
- Verified end to end against Ubuntu 24.04 sshd containers: key + encrypted key, password source,
  password target (relay), isolated target (auto → relay), pin mismatch, preview with deletes,
  `max_delete`, and stop. macOS hosts are covered by the probe/flag logic and unit tests, not yet by a
  live run.

## Rejected

- **Store the target key on the source** (or require the operator to set up source → target keys).
  Leaves a long-lived credential on a host the orchestrator does not control.
- **Always relay.** Simple, but doubles traffic and needs staging disk for every profile.
- **rsync daemon (`rsync://`) on targets.** Another service and port per host, and its auth is weaker
  than SSH.
- **paramiko/SFTP instead of rsync.** No delta transfer and no rsync semantics (permissions, links,
  deletes, excludes).
