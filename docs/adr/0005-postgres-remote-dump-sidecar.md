# ADR 0005 — Postgres remote dump runs in a `cdc-pgdump` sidecar

Status: Accepted
Owner: TBD
Last updated: 2026-11-03

## Context

Operators need to (a) take a dump of a remote PostgreSQL database and download it, and (b) seed a
target from a source before starting CDC. Both are one-shot jobs that can run for hours and write
gigabytes. The other one-shot tools (pgcopydb, pgloader, mongocopy, RedisShake) already run as
sidecars behind `/cdc/<tool>/*`.

## Decision

1. **Sidecar, not in-process.** `docker/pgdump/` (stdlib Python `serve.py` on `postgres:<PG_MAJOR>`)
   runs `pg_dump` / `pg_restore`; the orchestrator only proxies `/cdc/pgdump/*` and stores profiles
   (rqlite kind `pgdump_profile`). Same contract as the other tools, so the registry, contract,
   and UI patterns are reused. The orchestrator stays a small control process (ADR 0001).
2. **Native client, not a JDBC re-implementation.** `pg_dump` preserves sequences, functions,
   grants and dependency order, and produces `pg_restore`-able archives. Client major is pinned by
   build arg `PG_MAJOR` and must be ≥ every source server.
3. **Two modes on one profile.** `dump` keeps a file for download; `dump_restore` always dumps
   `-Fc` then runs `pg_restore` (parallel with `restore_jobs`) and keeps the archive.
4. **Secrets never on argv.** A per-job `0600` `.pgpass` with exact `host:port:*:user` lines (the
   source password is never offered to the target), deleted at job end and on sidecar start.
   Inherited `PG*` env is dropped. `--dbname` for `pg_restore` is a quoted conninfo.
5. **Strict input.** Unknown option keys are rejected (a typo must not widen a dump); object
   patterns are allow-listed; `--strict-names` is added whenever schemas/tables are included.
6. **Schemas for table-filtered restores come from the archive TOC.** `pg_dump --table` omits
   `CREATE SCHEMA`; before `pg_restore`, schemas listed by `pg_restore --list` are created with
   `CREATE SCHEMA IF NOT EXISTS`. Derived from the archive, never guessed from patterns.
7. **Restore errors are reported, not hidden.** `pg_restore` skips past object errors and exits 1;
   that becomes `completed_with_errors` with `errors_ignored`, `first_error`, and a hint.
   `restore_exit_on_error` makes it fail fast.
8. **Downloads stream end to end** (sidecar → orchestrator → Nuxt) with `X-Dump-Sha256`. The
   orchestrator uses a client with only a connect timeout, because the shared client's total
   timeout would cut large files.

9. **Remote storage via SFTP destinations (added 2026-11-02).** Dump files can be stored on other
   servers: after the dump the sidecar uploads to each profile destination with paramiko
   (`<file>.part` → size check → atomic rename, plus `<file>.sha256`), then frees the local copy
   unless `keep_local`. Key or password auth; secrets live only in the run request (never on disk)
   and are redacted in responses; when an editor round-trips `********`, secrets are restored from
   the best-matching stored destination (host, user, port, dir, name, auth; position breaks ties).
   Host keys can be pinned by SHA256 fingerprint; unpinned uploads report the observed one.
   Rejected: running `pg_dump` on the DB host over SSH (needs shell + client on every DB host,
   impossible for managed Postgres) and shelling out to `sftp`/`sshpass` (passwords on the command
   line, weaker error reporting).

10. **Connection tests run where pg_dump runs (added 2026-11-03).** `POST /cdc/pgdump/test-connection`
    proxies to the sidecar's `POST /v1/test-connection`, which connects with `psql` using the same
    per-call `0600` `.pgpass` and cleaned `PG*` env as a job, and returns server version, client
    compatibility (`compatible: false` when the server is newer than `pg_dump`), database size and
    schemas with table counts and sizes (for the editor's schema picker). A failed connection is a
    normal answer (`ok: false`, `error`, `hint`). Testing from the orchestrator was rejected: it sits
    on a different network than the sidecar, so it can pass while the dump cannot connect.
11. **A saved password belongs to one login (added 2026-11-03).** A redacted `********` is replaced
    by the stored password only when engine, host (case-insensitive), port (engine default when
    empty) and username are unchanged — on update and on test-connection. Otherwise 400 "enter the
    password again", so editing the host cannot send a stored password to a different server.
    `src/saved_secret.rs` applies the same rule to pgcopy, pgloader, mongocopy and redisshake.
12. **Ad-hoc SFTP send of a finished run's file (added 2026-11-03).** `POST /cdc/pgdump/jobs/{id}/send`
    `{destination, profile_id?}` → sidecar `POST /v1/jobs/{id}/send` uploads the file still on the
    dump volume to one server given in the request, in the background, with the same paramiko
    uploader as §9. Nothing is saved to the profile; the credential-free result is appended to the
    job's `uploads` with `adhoc: true` (in-flight: `sending: true`, last entry). One send per job at
    a time (409); delete and retention sweep skip a job while it sends; `…/stop` stops a send; the
    local file is never removed by a send; 410 if the file is already gone. Ad-hoc entries are not
    part of the run result (status, upload step, problems ignore them). A redacted `********` is
    filled from the profile's saved server only for the same login (host case-insensitive, port,
    username, auth) — no positional fallback, unlike the editor round-trip in §9.
    Rejected: re-running the dump with a temporary destination (re-dumps a large database just to
    copy a file) and saving the server first (the request is explicitly one-off).

## Consequences

- Dump files live on volume `cdc-pgdump-data`; retention (`PGDUMP_RETENTION_HOURS`) and a free-space
  floor (`PGDUMP_MIN_FREE_MB`) bound disk use. No object-storage upload yet.
- Job state is per sidecar instance (single container in compose); a sidecar restart marks running
  jobs failed.
- Only PostgreSQL. MySQL (`mysqldump`) would be a sibling sidecar with the same contract.

## Rejected

- **Reuse the pgcopydb sidecar image.** It already ships pg_dump, but its API is clone-shaped and
  it puts the password in the argv URI (persisted in `job.json`). Not a model to extend.
- **Restore by streaming `pg_dump | pg_restore`.** Loses parallel restore and the downloadable
  archive; a failed restore could not be retried without re-dumping.
