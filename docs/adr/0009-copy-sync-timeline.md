# ADR 0009 — Copy & Sync timeline: an audit trail per profile

Status: Accepted
Owner: TBD
Last updated: 2026-11-06

## Context

The six Copy & Sync tools (pgcopy, pgdump, pgloader, mongocopy, redisshake, rsync) keep only the
newest N runs per profile in their sidecars (ADR 0006), and their handlers wrote audit lines to the
log stream only (`log_action`). When a profile started failing, nothing showed when it began, who
changed what just before, or why an old run failed once its log was pruned. The UI sent no user
identity, so the orchestrator could only record `bearer`.

## Decision

1. **Separate table `cdc_tool_events`** in rqlite (not `cdc_audit_events`): pipeline noise
   (`sync.interval_tick` …) shares that table's row cap and would evict run history. Retention by
   age and rows: `ORCH_TOOL_EVENTS_RETENTION_DAYS` (365), `ORCH_TOOL_EVENTS_MAX_ROWS` (200 000),
   applied hourly. Without rqlite, events live in memory (5 000, lost on restart); the API says
   `persisted: false` and the UI warns.
2. **Handlers record** profile created / updated / deleted, run started or refused (`run.start_failed`,
   with the reason), stop requested, connection tested (saved profiles only). Updates store a
   leaf-level diff `{field, before, after}`; secret-looking keys appear only as `{field, secret: true}`.
3. **A run watcher records outcomes.** Every `ORCH_TOOL_EVENTS_POLL_SECS` (15) it reads each
   configured sidecar's job list and records `run.finished` (status, exit code, error, duration),
   stamped with the job's own `started_at` / `finished_at`, so the timeline shows when things
   happened rather than when they were noticed. It also records `run.started` for runs it did not
   see start (other instances, restarts).
4. **Deterministic ids for run events** (UUID v5 of tool + job + kind) with `INSERT OR IGNORE`:
   polls, restarts and the handler/watcher race never duplicate an entry; the first writer wins
   (the handler's, which carries the user).
5. **Failed and stopped runs keep their log tail** (last 25 lines, ANSI stripped, `user:pass@` in
   URIs masked, ≤ 6 000 chars). When the sidecar gives no `error` (pgcopydb exit 12), the first
   ERROR/FATAL line of the tail becomes the reason in the summary.
6. **No secrets.** Snapshots and job details are redacted by key name, and run details copy a
   whitelist of job fields (connection objects and argv are never stored).
7. **Who did it.** The native UI forwards the signed-in user (email, else user id) as `x-user` on
   every orchestrator call; `actor_from_headers` already reads it. API callers holding the bearer
   token can set `x-user` themselves — they are already trusted admins.
8. **API** `GET /cdc/tool-events?tool=&profile_id=&kind=<prefix>&status=&since=&until=&limit=`,
   newest first. **UI**: a Timeline tab on every profile (day groups, filters, search, date range,
   CSV export, links to runs the sidecar still keeps).

## Consequences

- The watcher makes one `GET /v1/jobs` per sidecar per poll, plus one log read per newly finished
  failed/stopped run (and once per retained failed run after an orchestrator restart).
- Timestamps are normalised to UTC milliseconds with `Z`, so text order is time order in SQL.
- Events outlive their profile (deleted profiles keep their history under the stored name).

## Rejected

- **Reuse `cdc_audit_events`.** Shared row cap; pipeline ticks would push run history out.
- **Ask the sidecars to keep more runs.** Grows their volumes with logs and staging; still lost on
  volume reset, and no record of profile changes or refused starts.
- **Record outcomes only when the UI polls a run.** Runs nobody watched would never be recorded.
