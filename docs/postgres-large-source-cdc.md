# CDC from a large PostgreSQL source

How to replicate a large, WAL-enabled PostgreSQL source into another PostgreSQL database: first
load the existing data, then stream the changes, without losing anything in between.

Related: [ADR 0014](adr/0014-cdc-start-position.md) (start position),
[ADR 0005](adr/0005-postgres-remote-dump-sidecar.md) (pg_dump sidecar),
[ADR 0006](adr/0006-copy-sync-sidecars.md) (pgcopydb),
[ADR 0019](adr/0019-live-source-cdc-readiness.md) (readiness check), and
[ssh-tunnel-sources.md](ssh-tunnel-sources.md) when the source is reachable only over SSH or
Cloudflare.

- [The rule](#the-rule)
- [Choose an approach](#choose)
- [Step 0: readiness check](#readiness)
- [Option C step by step](#option-c)
- [Risks on a large source](#risks)
- [Large dump over SSH](#large-dump-over-ssh)

<a id="the-rule"></a>
## The rule

**Create the replication slot *before* taking the backup.** A logical slot keeps WAL only from the
moment it is created. "Back up first, set up CDC later" silently loses every change made in
between. The orchestrator refuses a custom LSN older than "now" when the slot does not exist yet
(ADR 0014 §5).

Replaying a few changes twice is harmless: the JDBC sink upserts by primary key, so the target ends
up in the same state. Missing changes is never safe. When in doubt, start earlier.

<a id="choose"></a>
## Choose an approach

| | A. Debezium snapshot (`snapshot_then_stream`) | B. `pgcopydb clone --follow` | **C. Slot first → pg_dump/restore → Debezium from LSN** |
|---|---|---|---|
| Effort | Lowest (the default) | Low | Medium |
| Speed on large data | Slow: rows go through Kafka | Fastest: parallel COPY + indexes | Fast: `pg_restore -j` |
| WAL held during the copy | Yes, for the whole snapshot | Yes | Yes, for the dump + restore |
| Ongoing CDC runs on | Kafka/Debezium | pgcopydb's own apply (no Kafka) | Kafka/Debezium |
| Use when | Up to roughly tens of GB | One-time migration / cutover | **Large source + long-running pipeline** |

<a id="readiness"></a>
## Step 0: readiness check

Run this before creating a slot. It is read-only and never changes the source.

**UI:** Pipeline wizard → Source step → Capture mode *Logical*. Pick the tables, set the publication
and slot if you don't use the defaults, then click **Check source**. Problems come first, each with
its fix for your platform, followed by a copyable fix script.

**API:**

```bash
TOKEN=$(docker exec cdc-orchestrator printenv ORCH_API_TOKEN)
curl -s localhost:9788/cdc/profiles/catalog/source/readiness \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{
    "source_type": "postgres", "capture_mode": "logical",
    "schema_mode": "specific_schemas", "schemas": ["public"],
    "tables": ["public.orders", "public.order_items"],
    "connection": { "host": "src-host", "port": "5432", "database": "appdb",
                    "username": "cdc_user", "password": "…",
                    "publication": "dbz_publication", "slot": "cdc_mypipe" }
  }' | jq '{ready, platform, summary, problems: [.checks[] | select(.status!="pass") | {id,status,detail,fix}]}'
```

- For a saved pipeline, send `"profile_id"` and leave out the password.
- `jq -r .fix_script` prints the script to hand to a DBA.
- If the source is behind SSH, add the `ssh_tunnel_id` keys
  ([sample](ssh-tunnel-sources.md#api-samples)).

| Result | Meaning |
|---|---|
| `wal_level` fail | Needs `wal_level=logical` and a restart (RDS/Aurora: `rds.logical_replication=1`). |
| Free slot / WAL sender fail | Raise `max_replication_slots` / `max_wal_senders` (restart). |
| REPLICATION login fail | `ALTER ROLE cdc_user REPLICATION;` (RDS: `GRANT rds_replication`). |
| Publication missing | Expected before step 1. The connector never creates it (`autocreate=disabled`). |
| Replica identity / no PK | Fix before the dump; otherwise replayed updates/deletes cannot converge. |
| `max_slot_wal_keep_size` | Decide it now: too low and the slot is invalidated mid-restore. |
| `unknown` | The login cannot read the value. Have a DBA confirm it; this does not block `ready`. |

The probes run inside `cdc-jdbc-admin` with the pipeline's own login. An old jdbc-admin image
returns 404 on `/v1/probe`: rebuild it with `./docker/deploy.sh`.

**Manual SQL** (when jdbc-admin is not available):

```sql
SHOW wal_level; SHOW max_replication_slots; SHOW max_wal_senders; SHOW max_slot_wal_keep_size;
SELECT rolreplication FROM pg_roles WHERE rolname = current_user;
SELECT * FROM pg_publication_tables WHERE pubname = 'dbz_publication';
SELECT c.oid::regclass FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind = 'r' AND n.nspname NOT IN ('pg_catalog','information_schema')
  AND c.relreplident <> 'f'
  AND NOT EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = c.oid AND i.indisprimary);
SELECT slot_name, active, pg_size_pretty(pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)) FROM pg_replication_slots;
```

Suggested order: check → fix every `fail` → create the publication → check again → create the slot
→ check again (the slot row shows it) → dump.

<a id="option-c"></a>
## Option C step by step

**1. Create the publication and the slot**, using the names the pipeline will use (set
`connection.slot` explicitly):

```sql
CREATE PUBLICATION dbz_publication FOR TABLE public.t1, public.t2;   -- or FOR ALL TABLES
SELECT * FROM pg_create_logical_replication_slot('cdc_mypipe', 'pgoutput');   -- note the lsn
```

**2. Dump and restore** with a pgdump profile in `dump_restore` mode (`restore_jobs` ≈ CPU cores),
or with `pg_dump`/`pg_restore` yourself. Load indexes and FKs last (`pre-data` → data →
`post-data`).

**3. Start the pipeline from the slot:**

```json
"source": { "capture_mode": "logical", "start_position": { "mode": "earliest_retained" } }
```

`earliest_retained` resolves to the slot's `confirmed_flush_lsn`. A custom
`{"kind":"postgres_lsn", …}` position works too. The connector is created stopped, its offset is
seeded, and then it is resumed with `snapshot.mode=no_data`.

**4. Verify:** lag drops to zero, **Verify Record** (ADR 0013) shows no differences, and
`GET /cdc/profiles/{id}/position` shows the committed LSN.

**Strictly consistent variant:** create the slot over the replication protocol with
`EXPORT_SNAPSHOT`, then run `pg_dump --snapshot=<id>`. This matters only for tables without a
primary key; `pgcopydb --follow` does it automatically.

<a id="risks"></a>
## Risks on a large source

- **WAL builds up on the source** while the slot waits. Size the disk for (write rate × dump +
  restore time), and watch `pg_replication_slots`.
- **Do not drop or reuse the slot** between steps 1 and 3, and never start that slot with
  `snapshot_then_stream`.
- **Long dumps** hold back the source's xmin (vacuum). Run them off-peak.
- **DDL is not carried by pgoutput.** Freeze schema changes until CDC has caught up.

<a id="large-dump-over-ssh"></a>
## Large dump over SSH

Through an SSH tunnel (especially through Cloudflare), all traffic is one TCP stream, and
`pg_dump -Fc` uses a single connection. For a large source:

1. Create the slot first (step 1).
2. Run `pg_dump -Fd -j N` **at the site**.
3. Copy the directory to our side (File sync through the same tunnel, or any transfer).
4. Run `pg_restore -j N` locally, then continue with step 3.

The slot holds WAL during the transfer; watch its size.
