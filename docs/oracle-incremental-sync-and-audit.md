# Oracle incremental replica (PM_PRODUCT) and sync audit trail

This document summarizes the **incremental JDBC replica** pattern for Oracle sources without triggers or redo/log-based CDC, and the **sync audit trail** implemented in hyper-cdc-orchestrator (control actions, poll intervals, connector health, investigation APIs).

Related code:

| Area | Path |
|------|------|
| Incremental capture | `src/incremental_cdc.rs` |
| Poll interval audit | `src/watcher/poll_audit.rs` |
| Connector transition audit | `src/watcher/dispatch.rs` |
| Audit emit + snapshots | `src/audit.rs` |
| rqlite persistence | `src/persist.rs` (`cdc_audit_events`) |
| Profile actions + audit | `src/handlers.rs` |
| Audit / investigate API | `src/ops.rs` |
| Compose env defaults | `docker/docker-compose.yaml` |

---

## 1. Problem: Oracle production without redo or triggers

For branch OLTP Oracle (example: `PANDORA.PM_PRODUCT`):

- No physical deletes on the table (soft cancel flags only).
- No permission for Data Pump, LogMiner, or trigger-based CDC.
- Need low impact on the source during business hours.
- Need a **last sync watermark** so polls do not full-scan the table every time.

**Solution in this stack:** `pattern=source_to_target`, `capture_mode=incremental`.

- **Not** Debezium (`logical`) and **not** `sync_once` (both assume log-based capture on Oracle).
- JDBC Source polls on a **timestamp / upsert key** watermark; JDBC Sink **upserts** to the target.
- Hard deletes are **not** replicated (not required when the source never deletes rows).

---

## 2. PM_PRODUCT pipeline settings

Example table: `PANDORA.PM_PRODUCT` (~3.2M rows), PK `(branch_no, pro_code)`, index on `update_date`.

| Setting | Value |
|---------|--------|
| Pattern | `source_to_target` |
| Capture | `incremental` |
| Tables | `PANDORA.PM_PRODUCT` |
| `timestamp_column` | `UPDATE_DATE` |
| `upsert_key_columns` | `BRANCH_NO,PRO_CODE` |
| `incrementing_column` | omit (composite PK) |
| Steady-state schedule | `delivery_mode=micro_batch`, `interval_seconds=300` (or 600+) |
| Backfill schedule | `delivery_mode=realtime` (poll ~1s) during off-hours only |

**Oracle user:** read-only `SELECT` on `pandora.pm_product`.

**Pre-flight SQL:**

```sql
SELECT COUNT(*) FROM pandora.pm_product WHERE update_date IS NULL;
```

Rows with NULL `update_date` are skipped or problematic for JDBC timestamp mode (`validate.non.null=true` on the source connector).

**Oracle → Postgres:** incremental mode lowercases column names on the wire for Postgres targets (`needs_lowercase_fields` in `incremental_cdc.rs`).

---

## 3. Baseline (~3M rows) without DBA tools

Incremental polling is for **deltas**, not a one-shot bulk load.

| Approach | Notes |
|----------|--------|
| **Off-hours `realtime` backfill** | Start/resume after 22:00; ~500 rows/poll → roughly 2–4 hours for 3.2M if Oracle keeps up. Pause before branches open. |
| **Schema Compare copy** | OK for **pilot** (`max_rows` ≤ 100,000); not for full 3.2M via repeated resync (INSERT from row 0, cap per call). |
| **Branch-by-branch script** | `SELECT … WHERE branch_no = ?` with write access only on Postgres target. |

**Do not** use `micro_batch` + 300s for initial fill (weeks to catch up).

**Multi-night backfill:** `pause` at 07:30, `resume` at 22:00. Connect **offsets are kept**; no full re-read of already synced keys.

---

## 4. Night-window operations (no per-interval API calls)

| Time | Action | Audit `action` |
|------|--------|----------------|
| 22:00 | `POST …/profiles/{id}/actions/resume` (or `start` first night) | `profile.resume` / `profile.start` |
| While running | automatic | `sync.interval_tick`, optionally `sync.watermark_advanced` |
| 07:30 | `POST …/profiles/{id}/actions/pause` | `profile.pause` |

After baseline, switch to `micro_batch` + `interval_seconds=300` and **restart** once.

---

## 5. Sync audit trail (implementation)

Every structured audit goes through `audit::emit()`:

1. **Structured log** — tracing target `audit_event`
2. **rqlite** — table `cdc_audit_events` (when `ORCH_RQLITE_URL` is set)
3. **Redis** — list `cdc:audit:events` (when `ORCH_AUDIT_REDIS_URL` is set)

### 5.1 Audit actions

| Category | `action` | When |
|----------|----------|------|
| Config | `profile.create`, `profile.update`, `profile.delete`, `profile.enable` | Profile CRUD / enable |
| Control | `profile.start`, `profile.stop`, `profile.pause`, `profile.resume`, `profile.restart` | Profile actions |
| Control | `profile.test_initial_sync` | `start` / `restart` with `row_limit` |
| Control | `profile.resync` | Resync (`clear_target`, `row_limit` in detail) |
| Sync | `sync.interval_tick` | Each JDBC poll interval while **Running** (incremental/trigger replica) |
| Sync | `sync.watermark_advanced` | Source connector **offsets changed** since previous tick |
| Connector | `connector.failed`, `connector.paused`, `connector.running`, `connector.state_changed` | Kafka Connect state transitions |
| Errors | `sync.dlq_growth` | DLQ topic high-watermark increased |

`outcome` is `success` or `failure`. Profile control failures include `error` and a **profile snapshot** in `detail` (JSON).

### 5.2 rqlite schema

Created on orchestrator startup (`persist::ensure_schema`):

```sql
CREATE TABLE IF NOT EXISTS cdc_audit_events (
  audit_id TEXT PRIMARY KEY,
  timestamp TEXT NOT NULL,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  outcome TEXT NOT NULL,
  detail TEXT NOT NULL,
  profile_id TEXT,
  profile_name TEXT
);
```

Indexes: `timestamp`, `action`, `profile_id`.

Retention: after each insert, oldest rows are trimmed to **`ORCH_AUDIT_RQLITE_MAX_ROWS`** (default 200,000).

### 5.3 Environment variables (Docker defaults)

| Variable | Default (compose) | Purpose |
|----------|-------------------|---------|
| `ORCH_RQLITE_URL` | `http://cdc-rqlite:4001` | Required for durable audit DB |
| `ORCH_AUDIT_REDIS_URL` | `redis://cdc-redis:6379/1` | Optional Redis tail |
| `ORCH_AUDIT_REDIS_KEY` | `cdc:audit:events` | Redis list key |
| `ORCH_AUDIT_REDIS_MAX_LEN` | `100000` | Redis list trim |
| `ORCH_AUDIT_RQLITE_MAX_ROWS` | `200000` | rqlite row cap |
| `ORCH_POLL_AUDIT_CHECK_SECS` | `10` | How often poll-audit scheduler runs |

### 5.4 Limitations

- **`sync.interval_tick`** reflects the profile **schedule** and logs Connect **offsets**; it is not a hook inside the JDBC task. Use **`sync.watermark_advanced`** to see watermark movement.
- Default **Redis** in compose has no AOF; use rqlite as system of record or export for long compliance retention.
- **`profile.history`** on the profile document is a separate, capped lifecycle log—not the full audit stream.

---

## 6. HTTP API (investigation)

Base: `http://127.0.0.1:9788/cdc` (Bearer `ORCH_API_TOKEN` on mutating and data routes).

### 6.1 One-shot triage

```http
GET /cdc/profiles/{id}/investigate
```

Returns:

- `profile` — snapshot (status, capture mode, connectors, last error, interval)
- `connectors.source` — status + offsets
- `connectors.sink` — status
- `recent_audit` — last 100 rows for this profile
- `profile_history` — capped history on the profile

### 6.2 Query audit

```http
GET /cdc/audit/events?limit=100
GET /cdc/audit/events?profile_id={uuid}&action=sync.interval_tick&since=2026-10-30T22:00:00Z
GET /cdc/audit/events?outcome=failure&limit=50
```

Query parameters: `limit` (1–2000), `profile_id`, `action`, `outcome`, `since`, `until` (RFC3339 timestamps compared as strings on `timestamp` column).

### 6.3 Profile actions (audited)

```http
POST /cdc/profiles/{id}/actions/start
POST /cdc/profiles/{id}/actions/pause
POST /cdc/profiles/{id}/actions/resume
POST /cdc/profiles/{id}/actions/stop
POST /cdc/profiles/{id}/actions/restart
POST /cdc/profiles/{id}/actions/resync?clear_target=true&row_limit=10000
POST /cdc/profiles/{id}/actions/start?row_limit=5000
```

Resync with `row_limit` on incremental: direct Oracle→Postgres sample copy (max 100,000 per call); does **not** reset JDBC offsets. Resync without `row_limit`: resets source connector offsets only (not a full table bulk copy).

---

## 7. MCP pipeline create (prompt-friendly)

Use `cdc_pipeline_create` on `mcp-cdc` (writes need `ALLOW_WRITE_TOOLS=true`):

```json
{
  "name": "pandora-pm-product",
  "pattern": "source_to_target",
  "capture_mode": "incremental",
  "source_type": "oracle",
  "source_host": "<host>",
  "source_port": "1521",
  "source_database": "<service>",
  "source_username": "cdc_reader",
  "source_password": "<secret>",
  "schemas": ["PANDORA"],
  "tables": ["PANDORA.PM_PRODUCT"],
  "timestamp_column": "UPDATE_DATE",
  "upsert_key_columns": "BRANCH_NO,PRO_CODE",
  "target_type": "postgres",
  "target_host": "<target-host>",
  "target_port": "5432",
  "target_database": "<db>",
  "target_username": "<user>",
  "target_password": "<secret>",
  "delivery_mode": "micro_batch",
  "interval_seconds": 300,
  "start": false
}
```

Validate: `cdc_source_validate`, `cdc_target_validate`, then `cdc_profile_action` start/resume.

---

## 8. Triage playbook (when sync looks wrong)

1. `GET /cdc/profiles/{id}/investigate` — status, offsets, last errors.
2. `GET /cdc/audit/events?profile_id={id}&outcome=failure` — failed control or enable.
3. `action=connector.failed` — task errors in `detail`.
4. `action=sync.dlq_growth` — sink rejecting rows (schema/type/PK).
5. `action=sync.watermark_advanced` — if missing during known source changes, check `UPDATE_DATE` maintenance or NULL watermarks on Oracle.
6. Compare target `COUNT(*)` to source; spot-check `(branch_no, pro_code)`.

---

## 9. Deploy after code changes

```bash
cd docker
docker compose build cdc-orchestrator
docker compose up -d cdc-orchestrator
```

Confirm rqlite is up (`ORCH_REQUIRE_PERSISTENCE` in compose). Existing rqlite volumes gain `cdc_audit_events` on next orchestrator start.

**Read audit via Redis (optional):**

```bash
redis-cli -p 26379 LRANGE cdc:audit:events 0 20
```

**Read audit via rqlite (debug):**

```bash
curl -sS -XPOST 'http://127.0.0.1:14001/db/query' \
  -H 'Content-Type: application/json' \
  -d '[["SELECT action, outcome, timestamp FROM cdc_audit_events ORDER BY timestamp DESC LIMIT 20"]]'
```

---

## 10. References

- Design compendium: `docs/architecture/hyper-cdc-orchestrator-design.html` (patterns, MCP Ch.15)
- Agent rules: `AGENTS.md`, `.cursor/skills/cdc-mcp/SKILL.md`
- Install / compose: `INSTALL.md`, `docker/docker-compose.yaml`

---

## 11. Web UI — live sync audit

Kafbat native UI: **Operations → Sync audit** (`/audit`).

- Polls `GET /cdc/audit/events` every **3 seconds** when **Live** is on.
- Filters: outcome, action preset, pipeline, free-text search.
- Expand a row for JSON `detail`; click pipeline name for **Investigate** snapshot.

Requires orchestrator with rqlite audit table and rebuilt `cdc-orchestrator` + `cdc-native-ui` (or kafbat-webui) images.

