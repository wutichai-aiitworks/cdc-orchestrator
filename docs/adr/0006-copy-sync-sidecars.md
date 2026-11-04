# ADR 0006 — Copy & Sync sidecars: run history, connection tests, no secrets on argv

Status: Accepted
Owner: TBD
Last updated: 2026-11-03

## Context

pgcopy (pgcopydb), pgloader, mongocopy (mongodump/mongorestore) and redisshake (RedisShake) sidecars
each carried their own copy of a minimal job runner. Their job id was the profile id, so every run
overwrote the previous one (no history) and reused its work directory: after a failed pgcopydb run a
plain re-run picked up the dead snapshot and always failed. Connection checks only validated fields.
pgcopydb received passwords inside its `--source/--target` URIs, which reached `job.json` and the run
log shown in the UI. The pgcopydb image (`dimitri/pgcopydb:latest`) shipped only a PostgreSQL 16
client, so it could not copy PostgreSQL 17+ sources.

## Decision

1. **Shared code, per-tool images.** `docker/sidecar_common/` (job store, HTTP contract, probes) is
   copied into each image with compose `additional_contexts: common`; build contexts stay per tool.
   Each `serve.py` keeps only what is tool-specific: building the command, the test, capability.
2. **One job per run.** The sidecar assigns the job id and stores `profile_id` (orchestrators that still
   send `id` are treated as sending the profile id). One active run per transfer (409); the newest N runs
   per transfer are kept. Old jobs whose id is a profile id still list under that profile.
3. **Runs start clean.** A new run has a new directory; pgcopydb's "resume" explicitly moves the previous
   run's state into the new run.
4. **Results are finalised when the process exits** (a watcher thread), not on the next status poll, so
   per-run secret files are deleted immediately.
5. **No secrets on argv, in job.json or logs.** pgcopydb: URIs without passwords + 0600 `PGPASSFILE`.
   pgloader/mongocopy/redisshake: their config files (which must hold the URI or password) are 0600 and
   deleted at the end of the run; mongodump/mongorestore read the URI from `--config`.
6. **Test connection runs in the sidecar**, where the tool runs: psql (pgcopydb; pgloader target),
   `pgloader --dry-run` (MySQL/SQL Server source: the same driver a load uses; the dummy target's result
   is ignored), Python sqlite3/csv (file sources), mongosh with the URI in the environment, and raw RESP
   (AUTH/PING/INFO) for Redis. A failed connection is a normal answer with a hint, not an HTTP error.
   The orchestrator fills a redacted password only for the saved login (ADR 0005 §11).
7. **pgcopydb image = `postgres:<PG_MAJOR>` + pgcopydb from apt.postgresql.org** (`CDC_PGCOPY_PG_MAJOR`,
   default 18), the same pinning as `cdc-pgdump`. The image's pg_dump must be ≥ every source server; the
   test warns when it is not.

## Consequences

- Each sidecar volume now holds up to N run directories per transfer (small: logs and metadata;
  mongocopy archives and RedisShake staging are deleted at the end of each run).
- Building a sidecar image outside compose needs `--build-context common=docker/sidecar_common`.
- The demo databases stay on PostgreSQL 17; upgrading them is a separate decision.

## Rejected

- **Keep the profile id as job id and add a history file.** Runs would still share a work directory
  and the stale-state failure would remain.
- **pip-installed drivers (pymysql, pytds) for pgloader tests.** `pgloader --dry-run` uses the exact
  driver and auth plugins a load uses; extra drivers could pass while pgloader fails.
