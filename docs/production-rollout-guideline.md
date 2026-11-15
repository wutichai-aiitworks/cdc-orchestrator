# Production rollout guideline: large PostgreSQL + Redis real-time sync

Scope: one customer production source → target, with
**PostgreSQL → PostgreSQL** (large database, CDC) and **Redis → Redis** (real time). The goal is to
keep the impact on the production source low, and to schedule the heavy steps in the low-traffic
night window.

Related: [postgres-large-source-cdc.md](postgres-large-source-cdc.md) (Option C in detail),
[ssh-tunnel-sources.md](ssh-tunnel-sources.md) (if the source is reachable only over SSH or
Cloudflare), ADR 0014 (start position), ADR 0019 (readiness check).

- [1. Readiness verdict](#verdict)
- [2. What loads the source, and how much](#impact)
- [3. Day vs night](#schedule)
- [4. Runbook](#runbook)
- [5. Monitoring and abort criteria](#monitoring)
- [6. Rollback](#rollback)

---

<a id="verdict"></a>
## 1. Readiness verdict (as of 2026-11-15)

| Component | Status | Before going to the customer |
|---|---|---|
| PostgreSQL CDC (Debezium logical, Option C: slot first → dump/restore → start from LSN) | **Ready, with a rehearsal** | Start position from an LSN, the readiness check and auto-recover are built and verified on Postgres. Rehearse once on a staging copy of production size and write rate. |
| Readiness check, pg_dump sidecar, Verify Record | **Ready** | — |
| SSH tunnel / Cloudflare Access connection method | **Not yet**: built 2026-11-15 and verified only against a simulated SSH site; the Cloudflare path has not been run against a real Access app | Commit it, build the CI image, and test once against the customer's real Cloudflare app in staging. Use it only if the customer's source is closed to us. |
| Redis real-time sync (RedisShake `sync_reader`) | **Ready, with a policy choice** | A stopped real-time sync is **alerted** (rule condition `redis_sync_down`), and is **auto-restarted** per profile: Off / Night window (default for new profiles, 00:00-05:00 in `ORCH_TIME_ZONE_OFFSET` time) / Any time, with backoff and at most 3 restarts in 24 h (ADR 0022). Every restart is still a full snapshot on the source, so keep the night window. Set `ORCH_TIME_ZONE_OFFSET` (e.g. `+07:00`) and create a `redis_sync_down` alert rule. |
| Release state | **Not yet** | The work is on branch `test` and uncommitted; images were built only locally. Commit, run CI, deploy to staging, then production. |

**Go / no-go:** go once every item below is ticked.

- [ ] Code committed, CI green, images pushed and pinned in `images.env`
- [ ] Staging rehearsal of the full runbook on production-sized data (§4), with source metrics recorded
- [ ] Readiness check `ready: true` on the production source (no `fail`)
- [ ] Source disk headroom ≥ (peak WAL rate × planned load time × 2)
- [ ] Redis: PSYNC allowed on the source, and memory headroom checked (§2.2)
- [ ] Redis: auto-restart = Night window on the real-time profile, `ORCH_TIME_ZONE_OFFSET` set, a `redis_sync_down` alert rule with a channel
- [ ] An on-call person and the abort criteria (§5) agreed for the night window

---

<a id="impact"></a>
## 2. What loads the source, and how much

### 2.1 PostgreSQL

| Phase | What happens on the source | Load | When |
|---|---|---|---|
| **Enable logical decoding** (`wal_level=logical`, slot/sender limits) | Config change + **restart** (RDS/Aurora: parameter group + reboot) | Downtime of one restart; afterwards WAL is a little larger | **Maintenance window**, days before |
| **Create publication + slot** | Metadata only | ~0 | Any time, but right before the dump |
| **Slot waiting** (until CDC starts) | The source **keeps all WAL** from the slot's LSN | Disk grows by your write rate × hours | Keep the gap short; watch disk |
| **Initial copy** (pg_dump of the whole database) | Full read of every table (I/O + network), one long snapshot transaction (holds back vacuum → temporary bloat) | **Heavy**: the only heavy phase | **Night window** |
| **Restore on the target** | Nothing on the source | 0 on the source | Overnight / next day |
| **Streaming CDC** (steady state) | One walsender decodes WAL for the slot | **Low**: proportional to the write rate, typically a fraction of one CPU core; no queries on your tables | 24/7 |
| **Connector restart / catch-up** | Decoding the backlog faster for a while | Short, moderate | Any time |

Key points:

1. **Streaming CDC does not query your tables.** It reads the WAL that Postgres writes anyway. In
   normal operation the source impact is small and constant, so 24/7 real time is fine.
2. **The heavy part is the first full copy.** Run it at night. It is a one-time cost, and a re-copy
   is needed only after a long outage.
3. **The real production risk is disk, not CPU.** If CDC stops (target down, network, Kafka), the
   slot keeps WAL until the disk fills and **the production database stops**. Set
   `max_slot_wal_keep_size` (PG 13+) as a safety cap, and alert on slot lag (§5). If the cap is
   reached, the slot is invalidated and you need a new initial copy, but production keeps running.
4. **Lower the load of the copy further:**
   - Dump from a **physical standby** if the customer has one. This puts zero load on the primary.
     The slot stays on the primary. Before dumping, confirm the standby has replayed past the slot:
     `SELECT pg_last_wal_replay_lsn() >= '<slot lsn>'` on the standby.
   - Limit parallelism: `pg_dump -j 2` (not more) on a busy primary; the restore side can use more.
   - Exclude large tables that the target does not need (pgdump profile `include_tables` / schemas).
5. **Tables without a primary key** need `REPLICA IDENTITY FULL`. That makes WAL for those tables
   larger, and it must be set *before* the dump.
6. **Freeze DDL** (schema changes) from the slot creation until CDC has caught up. pgoutput does
   not carry DDL.

### 2.2 Redis

The current sync uses RedisShake `sync_reader`: the sync connects to the source **as if it were a
replica** (`PSYNC`).

| Phase | What happens on the source | Load | When |
|---|---|---|---|
| **Full snapshot** (at every start) | The source runs **BGSAVE**: it forks, writes an RDB (to disk, or sent directly if `try_diskless`), then streams it | **Heavy for a short time**: the fork stalls the main thread (≈10–20 ms per GB of memory); memory can grow up to **2×** used memory under heavy writes (copy-on-write); disk and network I/O | **Night window** |
| **Live stream** (steady state) | Sends every write command to the sync, like to one replica | **Very low** | 24/7 |
| **Any restart** of the sync or its sidecar | **A new full snapshot** (no checkpoint resume) | Heavy again | Restart only at night: auto-restart "Night window" does this for you (ADR 0022) |

Key points:

1. **Check that PSYNC is allowed before planning.** Managed Redis (AWS ElastiCache/MemoryDB, Azure
   Cache, GCP Memorystore) usually **blocks SYNC/PSYNC**, and the current sidecar does not support
   the alternative scan mode. Self-managed Redis on VMs or Docker works.
2. **Memory headroom:** free RAM on the source host ≥ `used_memory`, to be safe during BGSAVE. If
   the source has a **replica**, set `prefer_replica = true` so the snapshot is taken there.
3. **Avoid the full-sync loop.** During a big snapshot, the source buffers new writes for the
   replica. If that buffer exceeds `client-output-buffer-limit replica` (default `256mb 64mb 60`),
   the source drops the connection and the sync starts over, so the load repeats. For large or busy
   Redis, raise it before the first sync:
   ```
   CONFIG SET client-output-buffer-limit "replica 1gb 512mb 120"
   CONFIG SET repl-backlog-size 256mb
   ```
   (Also persist the settings in redis.conf or the parameter group.)
4. The target is overwritten by the snapshot. Use "empty target first" only for a dedicated target.

---

<a id="schedule"></a>
## 3. Day vs night

| Step | Window | Why |
|---|---|---|
| Postgres config change (`wal_level=logical`) | Maintenance window (restart) | Restart |
| Readiness check, publication, connectivity tests, tunnel setup | Day | Read-only / metadata |
| **Postgres: create slot → pg_dump** | **Night** (start at the beginning of the window) | Full read + long transaction |
| Postgres restore on the target | Night → day | Target only |
| Start Postgres CDC from the slot (`earliest_retained`) | Any time after the restore; ideally the same night or early morning | Catches up the WAL kept since the slot was created; moderate for a short time |
| **Redis: start the sync (snapshot)** | **Night** | BGSAVE fork + memory |
| Postgres + Redis steady-state streaming | **24/7** | Low load |
| Planned restarts / redeploys of `cdc-redisshake` | Night | Each restart = a new Redis snapshot (auto-restart with "Night window" waits for the window) |
| Restarts of Connect / orchestrator / `cdc-sshtunnel` | Any time | Postgres CDC resumes from its offset (no re-copy) |

Real time during the day is fine. Only the **one-time copy steps** and **Redis restarts** belong
in the night window.

---

<a id="runbook"></a>
## 4. Runbook

### T-7 days: preparation (day)

1. Get access: direct, VPN, or SSH tunnel (rehearsed in staging if used).
2. Run the **readiness check** (UI: pipeline wizard → Check source) and fix every `fail`. If
   `wal_level` needs a change, book the maintenance restart.
3. Measure the **source**: database size, peak WAL rate
   (`SELECT pg_size_pretty(pg_wal_lsn_diff(pg_current_wal_lsn(), '0/0'))` sampled 1 h apart),
   free disk. Size the slot cap:
   `max_slot_wal_keep_size` ≥ WAL rate × (dump + restore time) × 2, and below the free disk.
4. Tables without a PK → `ALTER TABLE … REPLICA IDENTITY FULL` (or add a PK).
5. Redis: `INFO replication`, `INFO memory`, and check PSYNC is allowed (a test sync against a
   staging target). Apply the buffer settings from §2.2.
6. **Rehearse the whole runbook in staging** on a production-sized copy; record the times and the
   source CPU, I/O and disk.

### Night 1: initial load

| Time | Step |
|---|---|
| 00:00 | Pre-check: source healthy, disk headroom, a backup exists, on-call ready |
| 00:05 | Create the publication + **slot** (`pg_create_logical_replication_slot`) and note the LSN |
| 00:10 | Start the **pg_dump** (pgdump profile `dump_restore`, or pg_dump at the site/standby, `-j 2`) |
| … | Monitor every 15 min (§5). Abort if a threshold is crossed |
| 00:30 | **Redis**: start the RedisShake sync (snapshot); watch memory and latency until the job reaches the live stream |
| dump done | Restore on the target (`pg_restore -j N`, indexes/FKs last) |
| restore done | Start the Postgres pipeline with `start_position = earliest_retained` |
| 05:00 latest | Hard stop: if the dump is not done, cancel it, **drop the slot** (it frees the WAL), and retry another night with a smaller scope or from a standby |

### Day 1: verify

1. Postgres: connector lag → 0; **Verify Record** for the key tables shows no differences;
   `GET /cdc/profiles/{id}/position` keeps advancing.
2. Redis: the job status is running and the key counts on source vs target are close
   (`DBSIZE`, sampled at the same time).
3. Source back to its normal CPU, I/O and disk; slot WAL retained is small (§5).

### Steady state

Monitor (§5). Do Redis sidecar restarts and redeploys only at night.

---

<a id="monitoring"></a>
## 5. Monitoring and abort criteria

**PostgreSQL source:**

```sql
-- WAL kept by our slot (alert > 5 GB, critical > 50% of free disk)
SELECT slot_name, active, wal_status,
       pg_size_pretty(pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)) AS retained
FROM pg_replication_slots WHERE slot_name = 'cdc_mypipe';

-- Long transactions (the dump's snapshot; vacuum is held back while it runs)
SELECT pid, now() - xact_start AS age, state, left(query, 60)
FROM pg_stat_activity WHERE xact_start IS NOT NULL ORDER BY xact_start LIMIT 5;
```

| Signal | Abort / act when |
|---|---|
| Source free disk | < 20%: stop the dump; < 10%: drop the slot |
| Source CPU / I/O wait during the dump | Above the agreed limit for > 10 min (e.g. CPU > 80%): cancel the dump and retry from a standby or with `-j 1` |
| Application latency (customer APM) | Any sustained regression: cancel |
| Slot `wal_status` | `unreserved` / `lost`: CDC must restart from a new copy |
| Pipeline status | `failed` that auto-recover does not clear in 10 min: investigate |

**Redis source:**

| Check | Command | Act when |
|---|---|---|
| Memory during the snapshot | `INFO memory` → `used_memory`, host free RAM | Host free RAM < 15%: stop the sync |
| Fork stall | `INFO stats` → `latest_fork_usec` | > 500 ms: next time use `prefer_replica` / a smaller instance window |
| Disconnect loop | Source log `client-output-buffer-limit` / sync log | Raise the buffer (§2.2) before retrying |
| Sync running | Alert rule `redis_sync_down`; Redis sync page; MCP `cdc_redis_sync_status` | Down: auto-restart handles it in the night window; "gave up" alert (3 tries in 24 h) → investigate, then start manually |

**From a chat client** (`mcp-cdc`, a read key is enough; see [mcp-tools.md](mcp-tools.md)):
`cdc_source_health` (status, latency, slot/WAL checks, tunnel) per pipeline, `cdc_ssh_tunnels`,
`cdc_redis_sync_status`. Daily prompt: *"Check production CDC: run cdc_source_health for every running
pipeline, cdc_ssh_tunnels and cdc_redis_sync_status. List only warnings, with what to do. Do not change
anything."*

Redis sync failures are alerted through rule condition **Redis sync stopped** (`redis_sync_down`):
create it under Alerts with the customer's channel before go-live. The same rule announces
auto-restarts, "gave up" and recovery.

---

<a id="rollback"></a>
## 6. Rollback

Nothing in this plan writes to the source except the publication, the slot, and optional
`REPLICA IDENTITY` / Redis buffer settings.

| To undo | Command |
|---|---|
| Stop Postgres CDC and free the WAL | **Delete** the pipeline: it removes its connectors and drops its replication slot (the dialog shows the slot and the WAL it holds). If the source is unreachable the delete is refused and the pipeline is kept, so no slot is orphaned. Only "Keep the replication slot" leaves it; then drop it yourself: `SELECT pg_drop_replication_slot('cdc_mypipe');` |
| Remove the publication | `DROP PUBLICATION dbz_publication;` |
| Stop the Redis sync | Redis sync page → Stop (the source only loses a replica connection) |
| Revert the Redis buffers | `CONFIG SET client-output-buffer-limit "replica 256mb 64mb 60"` |
| `wal_level` | Leave it at `logical` (harmless); reverting needs another restart |

**Never leave an unused slot behind:** it keeps WAL forever and will fill the disk.

- **Delete** drops the slot automatically (B-02).
- **Stop** deliberately *keeps* it, so the pipeline can resume without losing changes, and WAL
  builds up meanwhile. Don't leave a production pipeline stopped for long: delete it, or watch
  `pg.slot_lag` with `cdc_source_health`.
