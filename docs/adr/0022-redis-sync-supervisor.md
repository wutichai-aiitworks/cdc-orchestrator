# ADR 0022 — Redis sync supervisor: alert and auto-restart real-time syncs

Status: Accepted (built 2026-11-15; verified end to end against two Redis containers)
Owner: Wutichai

## Context

A real-time Redis → Redis sync (RedisShake `sync_reader`, ADR 0006) is a sidecar job, not a
supervised pipeline:

- When the sidecar restarts, the run is marked failed and nothing restarts it.
- Nothing alerts anyone.
- RedisShake keeps no checkpoint, so a restart must take a new **full snapshot** (BGSAVE fork + RDB
  transfer) on the production source. Restarting "live only" would silently lose every change made
  while the sync was down.

This blocked the enterprise rollout (docs/production-rollout-guideline.md §1).

## Decision

1. **A watcher loop** (`src/watcher/redis_supervisor.rs`, every 60 s) looks at every real-time
   profile (`options.sync_aof`) and its latest run:
   - **running** → nothing to do; if it had been down, it records "recovered".
   - **stopped** → a person stopped it: never alerted, never restarted.
   - **failed / lost / exited** → *down*.
2. **Every down run is alerted once**, whatever the policy:
   - audit entry `redisshake.down`;
   - rule condition **`redis_sync_down`**, where the filter is the profile name.

   Auto-restart, give-up and recovery send to the same condition (Info / Critical / Resolved). The
   audit entries carry **no `profile_id`**, so they never fire `pipeline_failed` rules (those
   belong to CDC pipelines).
3. **Auto-restart is a per-profile policy** (`options.auto_restart`):
   - `off` is the API default (existing profiles keep their behaviour).
   - `window` restarts only inside `restart_window`. The default is `00:00-05:00` in
     `ORCH_TIME_ZONE_OFFSET` time (e.g. `+07:00`; default UTC); the window may wrap past midnight.
     New profiles in the UI default to `window`.
   - `always` restarts at any time.

   Only real-time profiles may set it; a one-time copy is refused with 400.
4. **A restart reuses the run path** of the Start button (`redisshake_handlers::start_profile_run`,
   actor `system:watcher`), so it uses the profile's own mode: a snapshot when `sync_rdb` is on, plus
   `empty_target` if set. It is recorded in the Copy & Sync timeline like a manual run.
5. **Backoff and cap:** 10 min → 30 min → 90 min between restarts, at most **3 in 24 h**. After that
   it sends a "gave up" alert and waits for a person, so a sync that keeps failing cannot hammer the
   source with snapshots.
6. **Visibility:**
   - `GET /cdc/redisshake/profiles` adds `supervisor{down, restarts_last_24h, next_restart_after,
     gave_up}` and `time_zone_offset_minutes`.
   - MCP `cdc_redis_sync_status` shows the policy and this state.
   - The Redis sync page has an **Auto-restart** setting (Off / Night window / Any time + window).

## Consequences

- Supervisor state is in memory: after an orchestrator restart, a still-down sync is alerted once
  more, and the 24 h restart count starts again.
- One orchestrator instance supervises. Several instances would each restart (not supported, the
  same as the other watcher loops).
- Detection takes up to one tick (60 s). The restart then happens within the same tick when the
  policy allows it.
- Keys deleted on the source while the sync was down stay on the target after a restart, unless
  the profile has `empty_target` (the snapshot overwrites, it does not delete).

## Verified

- Unit tests: run-state mapping (`succeeded` counts as down for real-time), window with offset and
  midnight wrap, policy/backoff/cap, option validation, UTC offset parsing.
- End to end (two `redis:7-alpine` containers):
  - one-time copy + auto-restart refused;
  - snapshot and live sync OK;
  - after a sidecar restart the run failed, the supervisor started a new run within one tick, and
    a key written **while the sync was down** plus a later live key reached the target;
  - audit `redisshake.down` + `redisshake.auto_restart` with no `profile_id`;
  - a sync stopped by a person was not restarted.

## Rejected

- **Restart without a snapshot** (`sync_rdb=false`): loses the changes made during the outage.
- **`restart: unless-stopped` on the sidecar only:** the process comes back, the run does not.
- **Always restart immediately:** a full snapshot at peak hours is exactly the source impact the
  rollout guideline schedules at night.
