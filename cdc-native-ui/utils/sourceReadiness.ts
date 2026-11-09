// Source readiness guides shown in the pipeline wizard's Source step.
//
// Log-based CDC (Debezium) runs for PostgreSQL, MySQL, MariaDB, SQL Server and
// Oracle (see reconcile.rs `build_debezium_config`). DB2, Informix and SQLite
// are captured with triggers (trigger_cdc), so their guide covers that path.
// Keep the defaults here in step with the orchestrator: PostgreSQL publication
// `dbz_publication` (autocreate disabled), changelog `hyper_cdc*`, triggers
// `hyper_cdc_*`, and native changelog pruning only for PG/MySQL/MariaDB/SQL
// Server/SQLite.

export type ReadinessTier = "small" | "medium" | "large"

export const READINESS_TIERS: ReadinessTier[] = ["small", "medium", "large"]

export type ReadinessParam = {
  name: string
  values: Record<ReadinessTier, string>
  /** When the change takes effect, e.g. Restart, Reload, Online. */
  apply: string
  /** True when applying needs a restart or short outage (highlighted). */
  disruptive?: boolean
  info: string
}

export type ReadinessCheck = {
  key: string
  title: string
  sql: string
  ready: string[]
  extra?: { label: string; sql: string }
}

export type ReadinessGuide = {
  title: string
  summary: string
  /** Warning shown above the guide, e.g. capture mode not available. */
  modeNote?: string
  tierHelp: Record<ReadinessTier, string>
  params: ReadinessParam[]
  scriptLabel: string
  script: (tier: ReadinessTier) => string
  notes: (tier: ReadinessTier) => string[]
  checks: ReadinessCheck[]
}

export type ReadinessContext = {
  username: string
  /** `schema.table` entries selected in the wizard (may be empty). */
  tables: string[]
  database: string
  captureMode: string
}

export const LOG_CDC_SOURCES = ["postgres", "mysql", "mariadb", "sqlserver", "oracle"]
export const TRIGGER_ONLY_SOURCES = ["db2", "informix", "sqlite"]

export const splitTableList = (raw: string): string[] =>
  raw
    .split(/[\s,]+/)
    .map((t) => t.trim())
    .filter(Boolean)

const sqlString = (v: string) => `'${v.replace(/'/g, "''")}'`
const nString = (v: string) => `N'${v.replace(/'/g, "''")}'`
const dq = (v: string) => `"${v.replace(/"/g, '""')}"`
const bracket = (v: string) => `[${v.replace(/]/g, "]]")}]`

const parts = (qualified: string): [string, string] => {
  const i = qualified.lastIndexOf(".")
  return i < 0 ? ["", qualified] : [qualified.slice(0, i), qualified.slice(i + 1)]
}

const schemasOf = (tables: string[]) => [...new Set(tables.map((t) => parts(t)[0]).filter(Boolean))]

const pick = (params: ReadinessParam[], name: string, tier: ReadinessTier) =>
  params.find((p) => p.name === name)?.values[tier] ?? ""

const LOG_TIER_HELP: Record<ReadinessTier, string> = {
  small: "Under ~50 GB, light writes (< ~1 GB of log per hour), 1–2 pipelines on this server.",
  medium: "~50–500 GB, steady writes (a few GB of log per hour), up to ~5 pipelines or a replica.",
  large: "Over ~500 GB or heavy writes (10+ GB of log per hour), many pipelines, bulk loads or long transactions.",
}

const TRIGGER_TIER_HELP: Record<ReadinessTier, string> = {
  small: "A few captured tables with light writes. Realtime polling is fine.",
  medium: "Busy OLTP tables. Triggers add a changelog insert per row change, so log volume grows.",
  large: "High write rates or very wide rows. Poll on an interval and keep the changelog pruned.",
}

const triggerModeNote = (label: string, captureMode: string) =>
  captureMode === "logical"
    ? `CDC (WAL) is not available for ${label} in Hyper CDC. Switch Capture mode to Trigger (or Incremental). The checks below are for Trigger capture.`
    : undefined

const changelogParams = (): ReadinessParam[] => [
  {
    name: "Changelog retention (hours)",
    values: { small: "24", medium: "72", large: "72" },
    apply: "Pipeline",
    info: "How long rows stay in the trigger changelog after capture. It must cover your longest pipeline outage, or changes are lost before the connector reads them.",
  },
  {
    name: "Poll cadence",
    values: { small: "Realtime (1s)", medium: "Realtime (1s)", large: "Interval (60s+)" },
    apply: "Pipeline",
    info: "How often the JDBC source polls the changelog table (not your live tables). An interval lowers load on busy production sources.",
  },
]

// ---------------------------------------------------------------- PostgreSQL

const postgresGuide = (ctx: ReadinessContext): ReadinessGuide => {
  const params: ReadinessParam[] = [
    {
      name: "wal_level",
      values: { small: "logical", medium: "logical", large: "logical" },
      apply: "Restart",
      disruptive: true,
      info: "Enables logical decoding, which CDC (WAL) requires. 'replica' (the default) is not enough. Adds a little extra WAL for row identity.",
    },
    {
      name: "max_wal_senders",
      values: { small: "3", medium: "10", large: "20" },
      apply: "Restart",
      disruptive: true,
      info: "Concurrent replication connections. Each running pipeline uses one, and so does every physical standby and pg_basebackup. Keep it at least max_replication_slots + standbys + 1.",
    },
    {
      name: "max_replication_slots",
      values: { small: "3", medium: "10", large: "20" },
      apply: "Restart",
      disruptive: true,
      info: "One slot per CDC pipeline (named cdc_<topic prefix>), plus any physical standby slots. Leave headroom so a re-created pipeline can get a new slot before the old one is dropped.",
    },
    {
      name: "max_slot_wal_keep_size",
      values: { small: "1GB", medium: "16GB", large: "64GB" },
      apply: "Reload",
      info: "Most WAL a slot may hold back while its connector is stopped or lagging. Past this the slot becomes 'lost' and the pipeline must re-snapshot, but the disk is protected. -1 means unlimited, which can fill the disk. Size it to cover your longest expected outage at peak WAL rate.",
    },
    {
      name: "min_wal_size",
      values: { small: "80MB", medium: "1GB", large: "4GB" },
      apply: "Reload",
      info: "WAL kept for recycling instead of being removed after a checkpoint. A higher value avoids constantly creating and deleting segment files under steady write load.",
    },
    {
      name: "max_wal_size",
      values: { small: "1GB", medium: "4GB", large: "16GB" },
      apply: "Reload",
      info: "Soft limit before a checkpoint is forced. Too small causes frequent checkpoints (and 'checkpoints are occurring too frequently' warnings) under bulk writes and snapshots.",
    },
    {
      name: "logical_decoding_work_mem",
      values: { small: "64MB", medium: "128MB", large: "512MB" },
      apply: "Reload",
      info: "Memory per slot for decoding a transaction before it spills to disk (PostgreSQL 13+). Raise it for large transactions or bulk updates. Applies per walsender, so multiply by the number of slots.",
    },
  ]
  const quoted = new Set(["wal_level", "max_slot_wal_keep_size", "min_wal_size", "max_wal_size", "logical_decoding_work_mem"])
  const pub = "dbz_publication"
  const tables = ctx.tables.length ? ctx.tables : ["public.customers", "public.orders"]
  const role = ctx.username || "debezium"
  const roleId = dq(role)
  const schemas = schemasOf(ctx.tables)
  const grantSchemas = schemas.length ? schemas : ["public"]

  return {
    title: "PostgreSQL CDC readiness",
    summary: "Recommended WAL settings by database size, and the checks to run on the source before Start.",
    tierHelp: LOG_TIER_HELP,
    params,
    scriptLabel: "Settings SQL",
    script: (tier) =>
      [
        `-- ${tier[0].toUpperCase()}${tier.slice(1)} database — run as superuser, then restart PostgreSQL`,
        ...params.map((p) => {
          const v = p.values[tier]
          return `ALTER SYSTEM SET ${p.name} = ${quoted.has(p.name) ? `'${v}'` : v};`
        }),
        "",
        "-- Reload-only settings apply without a restart:",
        "SELECT pg_reload_conf();",
      ].join("\n"),
    notes: (tier) => [
      `Keep at least ${pick(params, "max_slot_wal_keep_size", tier)} + ${pick(params, "max_wal_size", tier)} free on the pg_wal volume per lagging slot, on top of normal growth.`,
      "Managed PostgreSQL (RDS, Aurora, Cloud SQL, Azure) does not allow ALTER SYSTEM. Set the same values in the parameter group or flags; on RDS and Aurora enable logical replication with rds.logical_replication = 1.",
      "Once a table is in a publication, UPDATE and DELETE on it fail on the source if it has no primary key. Give such tables a primary key, or set REPLICA IDENTITY FULL (or USING INDEX on a unique, NOT NULL index).",
    ],
    checks: [
      {
        key: "settings",
        title: "Server settings",
        sql: `SELECT
    name,
    setting,
    unit,
    pending_restart
FROM pg_settings
WHERE name IN (
    'wal_level',
    'max_wal_senders',
    'max_replication_slots',
    'max_slot_wal_keep_size',
    'min_wal_size',
    'max_wal_size',
    'logical_decoding_work_mem'
)
ORDER BY name;`,
        ready: [
          "wal_level = logical",
          "pending_restart = false on every row (otherwise restart PostgreSQL)",
          "Values at least the tier you picked above",
        ],
      },
      {
        key: "slots",
        title: "Replication slots",
        sql: `SELECT
    slot_name,
    plugin,
    slot_type,
    active,
    wal_status,
    pg_size_pretty(pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)) AS retained_wal
FROM pg_replication_slots;`,
        ready: [
          "Before the first start: no slot for this pipeline yet. Debezium creates cdc_<topic prefix> with plugin pgoutput.",
          "Running: slot_type = logical, active = true, wal_status = reserved or extended",
          "wal_status = unreserved is a warning: the slot is close to max_slot_wal_keep_size",
          "wal_status = lost: the slot is broken. Drop it and re-sync the pipeline.",
          "active = false with growing retained_wal: drop slots from deleted pipelines (SELECT pg_drop_replication_slot('name');)",
          "Fewer slots in use than max_replication_slots, so the next pipeline can start",
        ],
      },
      {
        key: "publication",
        title: "Publication",
        sql: `SELECT
    pubname,
    puballtables,
    pubinsert,
    pubupdate,
    pubdelete
FROM pg_publication
WHERE pubname = ${sqlString(pub)};`,
        ready: [
          `A row for ${pub} must exist. Hyper CDC does not create it (publication.autocreate.mode = disabled).`,
          "pubinsert, pubupdate and pubdelete = true",
          "puballtables = true, or every captured table listed in pg_publication_tables",
        ],
        extra: {
          label: "Create it if missing (pick one option)",
          sql: [
            "-- Option A: selected tables only (recommended; add new tables later with",
            `--           ALTER PUBLICATION ${pub} ADD TABLE schema.table;)`,
            `CREATE PUBLICATION ${pub}`,
            `FOR TABLE ${tables.join(", ")};`,
            "",
            "-- Option B: every table in the database, including tables created later",
            "--           (needs superuser; captures more WAL than you may need)",
            `CREATE PUBLICATION ${pub}`,
            "FOR ALL TABLES;",
          ].join("\n"),
        },
      },
      {
        key: "role",
        title: "Replication user",
        sql: `SELECT
    rolname,
    rolcanlogin,
    rolreplication
FROM pg_roles
WHERE rolname = ${sqlString(role)};`,
        ready: [
          "rolcanlogin = true and rolreplication = true",
          "Skip this when the CDC login is the database owner or a superuser",
          "pg_hba.conf must allow this user to connect from the Kafka Connect host",
        ],
        extra: {
          label: "Grant if missing",
          sql: [
            "-- Only when the CDC login is not the database owner",
            `ALTER ROLE ${roleId} WITH LOGIN REPLICATION;`,
            `GRANT CONNECT ON DATABASE ${dq(ctx.database || "<database>")} TO ${roleId};`,
            ...grantSchemas.flatMap((s) => [
              `GRANT USAGE ON SCHEMA ${dq(s)} TO ${roleId};`,
              `GRANT SELECT ON ALL TABLES IN SCHEMA ${dq(s)} TO ${roleId};`,
              `ALTER DEFAULT PRIVILEGES IN SCHEMA ${dq(s)} GRANT SELECT ON TABLES TO ${roleId};`,
            ]),
          ].join("\n"),
        },
      },
      {
        key: "pk",
        title: "Tables without a primary key",
        sql: `SELECT
    n.nspname AS schema_name,
    c.relname AS table_name,
    c.relreplident AS replica_identity
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind IN ('r', 'p')
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND n.nspname NOT LIKE 'pg_toast%'
  AND NOT EXISTS (
      SELECT 1 FROM pg_index i WHERE i.indrelid = c.oid AND i.indisprimary
  )
ORDER BY 1, 2;`,
        ready: [
          "No captured table listed, or each listed table has a unique NOT NULL index or an identity column (+ partition key)",
          "Hyper CDC derives the sink key from those. A table with neither blocks Start.",
          "Otherwise add a primary key, or set the source connection's message_key_columns (schema.table:col1,col2)",
        ],
      },
    ],
  }
}

// ---------------------------------------------------------------- MySQL / MariaDB

const mysqlFamilyGuide = (ctx: ReadinessContext, maria: boolean): ReadinessGuide => {
  const label = maria ? "MariaDB" : "MySQL"
  const params: ReadinessParam[] = [
    {
      name: "log_bin",
      values: { small: "ON", medium: "ON", large: "ON" },
      apply: "Restart",
      disruptive: true,
      info: maria
        ? "Binary logging is what Debezium reads. Off by default in MariaDB; set log_bin (and a non-zero server_id) in my.cnf."
        : "Binary logging is what Debezium reads. On by default in MySQL 8; also needs a non-zero server_id.",
    },
    {
      name: "binlog_format",
      values: { small: "ROW", medium: "ROW", large: "ROW" },
      apply: maria ? "Online + my.cnf" : "Online (SET PERSIST)",
      info: "Debezium needs row events. STATEMENT or MIXED formats are rejected. Sessions already open keep the old format until they reconnect.",
    },
    {
      name: "binlog_row_image",
      values: { small: "FULL", medium: "FULL", large: "FULL" },
      apply: maria ? "Online + my.cnf" : "Online (SET PERSIST)",
      info: "FULL writes every column for updates and deletes. MINIMAL leaves gaps the sink cannot upsert from.",
    },
    {
      name: "binlog_expire_logs_seconds",
      values: { small: "259200 (3d)", medium: "604800 (7d)", large: "604800 (7d)" },
      apply: maria ? "Online + my.cnf" : "Online (SET PERSIST)",
      info: `How long binlogs are kept. It must cover your longest connector outage; if the connector's position is purged, the pipeline must re-snapshot.${maria ? " MariaDB 10.6+; older versions use expire_logs_days." : ""} On large databases also budget the disk this takes at peak write rate.`,
    },
    {
      name: "max_binlog_size",
      values: { small: "100M", medium: "512M", large: "1G" },
      apply: maria ? "Online + my.cnf" : "Online (SET PERSIST)",
      info: "Size at which the server rotates to a new binlog file. Larger files mean fewer rotations under heavy writes.",
    },
    maria
      ? {
          name: "gtid_strict_mode",
          values: { small: "OFF", medium: "ON", large: "ON" },
          apply: "Online + my.cnf",
          info: "MariaDB always writes GTIDs. Strict mode keeps GTID order consistent so the connector can resume cleanly after a primary failover.",
        }
      : {
          name: "gtid_mode",
          values: { small: "OFF", medium: "ON", large: "ON" },
          apply: "Online, stepwise",
          info: "With GTIDs the connector can resume on a new primary after failover. Enable with enforce_gtid_consistency = ON, then step gtid_mode through OFF_PERMISSIVE → ON_PERMISSIVE → ON.",
        },
  ]
  const v = (name: string, tier: ReadinessTier) => pick(params, name, tier).split(" ")[0]
  const bytes: Record<string, string> = { "100M": "104857600", "512M": "536870912", "1G": "1073741824" }
  const user = ctx.username || "debezium"
  const account = `${sqlString(user)}@'%'`
  const schemas = schemasOf(ctx.tables)
  const schemaList = (schemas.length ? schemas : [ctx.database || "<database>"]).map(sqlString).join(", ")

  return {
    title: `${label} CDC readiness`,
    summary: "Recommended binlog settings by database size, and the checks to run on the source before Start.",
    tierHelp: LOG_TIER_HELP,
    params,
    scriptLabel: "Settings",
    script: (tier) =>
      maria
        ? [
            "# my.cnf [mysqld] — restart MariaDB after editing",
            "server_id                  = 1          # unique in your replication topology",
            "log_bin                    = mariadb-bin",
            "binlog_format              = ROW",
            "binlog_row_image           = FULL",
            `binlog_expire_logs_seconds = ${v("binlog_expire_logs_seconds", tier)}   # older than 10.6: expire_logs_days`,
            `max_binlog_size            = ${v("max_binlog_size", tier)}`,
            `gtid_strict_mode           = ${v("gtid_strict_mode", tier)}`,
            "",
            "-- Apply now without a restart (log_bin and server_id still need one):",
            "SET GLOBAL binlog_format = 'ROW';",
            "SET GLOBAL binlog_row_image = 'FULL';",
            `SET GLOBAL binlog_expire_logs_seconds = ${v("binlog_expire_logs_seconds", tier)};`,
            `SET GLOBAL max_binlog_size = ${bytes[v("max_binlog_size", tier)]};`,
            `SET GLOBAL gtid_strict_mode = ${v("gtid_strict_mode", tier)};`,
          ].join("\n")
        : [
            "-- my.cnf [mysqld] (restart required): server_id = 1, log_bin = mysql-bin",
            "SET PERSIST binlog_format = 'ROW';",
            "SET PERSIST binlog_row_image = 'FULL';",
            `SET PERSIST binlog_expire_logs_seconds = ${v("binlog_expire_logs_seconds", tier)};`,
            `SET PERSIST max_binlog_size = ${bytes[v("max_binlog_size", tier)]};`,
            ...(tier === "small"
              ? []
              : [
                  "",
                  "-- GTID: one step at a time; wait for Ongoing_anonymous_transaction_count = 0 before ON",
                  "SET PERSIST enforce_gtid_consistency = ON;",
                  "SET PERSIST gtid_mode = OFF_PERMISSIVE;",
                  "SET PERSIST gtid_mode = ON_PERMISSIVE;",
                  "SHOW GLOBAL STATUS LIKE 'Ongoing_anonymous_transaction_count';",
                  "SET PERSIST gtid_mode = ON;",
                ]),
          ].join("\n"),
    notes: () => [
      "Hyper CDC gives each connector a random database.server.id. It must not clash with a server_id in your replication topology.",
      maria
        ? "Managed MariaDB (RDS, Azure) sets these in the parameter group; enable automated backups so binary logging is on."
        : "Managed MySQL (RDS, Aurora, Cloud SQL, Azure) sets these in the parameter group or flags. On RDS also raise binlog retention: CALL mysql.rds_set_configuration('binlog retention hours', 168);",
      "The initial snapshot takes a brief global read lock (or table locks where that is not allowed). Run the first Start outside peak hours on large databases.",
    ],
    checks: [
      {
        key: "settings",
        title: "Server settings",
        sql: `SHOW GLOBAL VARIABLES
WHERE Variable_name IN (
    'server_id',
    'log_bin',
    'binlog_format',
    'binlog_row_image',
    'binlog_expire_logs_seconds',${maria ? "\n    'expire_logs_days',\n    'gtid_strict_mode'," : "\n    'gtid_mode',\n    'enforce_gtid_consistency',"}
    'max_binlog_size'
);`,
        ready: [
          "server_id is not 0 and log_bin = ON",
          "binlog_format = ROW and binlog_row_image = FULL",
          "Retention and binlog size at least the tier you picked above",
        ],
      },
      {
        key: "binlogs",
        title: "Binary logs",
        sql: "SHOW BINARY LOGS;",
        ready: [
          "At least one binlog listed (an error means binary logging is off)",
          "The oldest file covers your longest expected connector outage",
          "Total size fits comfortably on the binlog volume",
        ],
      },
      {
        key: "user",
        title: "Replication user",
        sql: `SHOW GRANTS FOR ${account};`,
        ready: [
          maria
            ? "SELECT, RELOAD, SHOW DATABASES, REPLICATION SLAVE (REPLICA) and BINLOG MONITOR (REPLICATION CLIENT before 10.5)"
            : "SELECT, RELOAD, SHOW DATABASES, REPLICATION SLAVE and REPLICATION CLIENT",
          "LOCK TABLES when the snapshot cannot take a global read lock (managed services)",
          "Grants are ON *.*. Replication privileges cannot be granted per database.",
        ],
        extra: {
          label: "Grant if missing",
          sql: [
            `CREATE USER IF NOT EXISTS ${account} IDENTIFIED BY '<strong password>';`,
            maria
              ? `GRANT SELECT, RELOAD, SHOW DATABASES, REPLICATION SLAVE, BINLOG MONITOR, LOCK TABLES ON *.* TO ${account};`
              : `GRANT SELECT, RELOAD, SHOW DATABASES, REPLICATION SLAVE, REPLICATION CLIENT, LOCK TABLES ON *.* TO ${account};`,
            "FLUSH PRIVILEGES;",
          ].join("\n"),
        },
      },
      {
        key: "pk",
        title: "Tables without a primary key",
        sql: `SELECT t.table_schema, t.table_name
FROM information_schema.tables t
LEFT JOIN information_schema.table_constraints c
       ON c.table_schema = t.table_schema
      AND c.table_name = t.table_name
      AND c.constraint_type = 'PRIMARY KEY'
WHERE t.table_type = 'BASE TABLE'
  AND t.table_schema IN (${schemaList})
  AND c.constraint_name IS NULL
ORDER BY 1, 2;`,
        ready: [
          "No captured table listed",
          "The target sink matches rows by key. Add a primary key to listed tables before Start.",
        ],
      },
    ],
  }
}

// ---------------------------------------------------------------- SQL Server

const sqlServerGuide = (ctx: ReadinessContext): ReadinessGuide => {
  const params: ReadinessParam[] = [
    {
      name: "SQL Server Agent",
      values: { small: "Running", medium: "Running", large: "Running" },
      apply: "Service",
      info: "The CDC capture and cleanup jobs are Agent jobs. Without Agent no changes reach the change tables. Express edition has no Agent.",
    },
    {
      name: "capture @maxtrans",
      values: { small: "500", medium: "1000", large: "5000" },
      apply: "Restart capture job",
      info: "Transactions processed per scan cycle. Raise it when the capture job falls behind the log under heavy writes.",
    },
    {
      name: "capture @maxscans",
      values: { small: "10", medium: "20", large: "50" },
      apply: "Restart capture job",
      info: "Scan cycles before the capture job pauses for @pollinginterval. Together with @maxtrans this caps throughput per pass.",
    },
    {
      name: "capture @pollinginterval (s)",
      values: { small: "5", medium: "2", large: "1" },
      apply: "Restart capture job",
      info: "Pause between passes. Lower means lower latency at a little more CPU.",
    },
    {
      name: "cleanup @retention (min)",
      values: { small: "4320 (3d)", medium: "10080 (7d)", large: "10080 (7d)" },
      apply: "Online",
      info: "How long change rows stay in the cdc.* tables. It must cover your longest connector outage, or the connector needs a new snapshot.",
    },
    {
      name: "CDC filegroup",
      values: { small: "PRIMARY", medium: "Dedicated", large: "Dedicated" },
      apply: "Per table",
      info: "Where change tables live (@filegroup_name in sp_cdc_enable_table). A dedicated filegroup keeps CDC I/O and growth away from your data files.",
    },
  ]
  const n = (name: string, tier: ReadinessTier) => pick(params, name, tier).split(" ")[0]
  const db = ctx.database || "<database>"
  const tables = ctx.tables.length ? ctx.tables : ["dbo.customers", "dbo.orders"]
  const user = ctx.username || "debezium"
  const enableTables = (filegroup: boolean) =>
    tables.map((t) => {
      const [s, name] = parts(t)
      return `EXEC sys.sp_cdc_enable_table @source_schema = ${nString(s || "dbo")}, @source_name = ${nString(name)}, @role_name = NULL, @supports_net_changes = 0${filegroup ? ", @filegroup_name = N'CDC'" : ""};`
    })

  return {
    title: "SQL Server CDC readiness",
    summary: "Change data capture setup and capture-job sizing, and the checks to run on the source before Start.",
    tierHelp: LOG_TIER_HELP,
    params,
    scriptLabel: "Setup SQL",
    script: (tier) =>
      [
        `USE ${bracket(db)};`,
        "-- 1. Enable CDC on the database (sysadmin)",
        "EXEC sys.sp_cdc_enable_db;",
        "",
        "-- 2. Enable CDC on each captured table (db_owner)",
        ...(tier === "small" ? [] : ["--    Create the CDC filegroup and a file in it first: ALTER DATABASE ... ADD FILEGROUP CDC;"]),
        ...enableTables(tier !== "small"),
        "",
        "-- 3. Size the capture and cleanup jobs",
        `EXEC sys.sp_cdc_change_job @job_type = N'capture', @maxtrans = ${n("capture @maxtrans", tier)}, @maxscans = ${n("capture @maxscans", tier)}, @pollinginterval = ${n("capture @pollinginterval (s)", tier)};`,
        `EXEC sys.sp_cdc_change_job @job_type = N'cleanup', @retention = ${n("cleanup @retention (min)", tier)};`,
        "EXEC sys.sp_cdc_stop_job @job_type = N'capture';",
        "EXEC sys.sp_cdc_start_job @job_type = N'capture';",
      ].join("\n"),
    notes: () => [
      "The transaction log cannot be truncated past changes the capture job has not read. If Agent stops, the log keeps growing (log_reuse_wait_desc = REPLICATION).",
      "After ALTER TABLE on a captured table, create a new capture instance for it (sp_cdc_enable_table with a new @capture_instance). The old one keeps the old columns.",
      "Azure SQL Database and Managed Instance support CDC with their own scheduler. Amazon RDS uses msdb.dbo.rds_cdc_enable_db instead of sp_cdc_enable_db.",
    ],
    checks: [
      {
        key: "database",
        title: "Database and Agent",
        sql: `SELECT name, is_cdc_enabled, recovery_model_desc, log_reuse_wait_desc
FROM sys.databases
WHERE name = DB_NAME();

SELECT servicename, status_desc
FROM sys.dm_server_services;`,
        ready: [
          "is_cdc_enabled = 1",
          "SQL Server Agent status_desc = Running",
          "log_reuse_wait_desc is not stuck on REPLICATION",
        ],
      },
      {
        key: "tables",
        title: "Captured tables",
        sql: `SELECT s.name AS schema_name, t.name AS table_name, t.is_tracked_by_cdc
FROM sys.tables t
JOIN sys.schemas s ON s.schema_id = t.schema_id
WHERE t.is_ms_shipped = 0
  AND s.name <> 'cdc'
ORDER BY 1, 2;`,
        ready: ["is_tracked_by_cdc = 1 for every table this pipeline captures"],
        extra: { label: "Enable CDC on the selected tables", sql: enableTables(false).join("\n") },
      },
      {
        key: "jobs",
        title: "Capture jobs",
        sql: `EXEC sys.sp_cdc_help_jobs;

SELECT TOP (20) *
FROM sys.dm_cdc_errors
ORDER BY entry_time DESC;`,
        ready: [
          "Both a capture and a cleanup job are listed",
          "maxtrans, maxscans, pollinginterval and retention match the tier above",
          "No recent rows in sys.dm_cdc_errors",
        ],
      },
      {
        key: "user",
        title: "CDC user",
        sql: `SELECT r.name AS role_name
FROM sys.database_role_members rm
JOIN sys.database_principals r ON r.principal_id = rm.role_principal_id
JOIN sys.database_principals m ON m.principal_id = rm.member_principal_id
WHERE m.name = ${nString(user)};`,
        ready: [
          "db_owner, or db_datareader plus SELECT on schema cdc",
          "If you set @role_name in sp_cdc_enable_table, the user must also be in that role",
        ],
        extra: {
          label: "Grant if missing",
          sql: [
            `-- CREATE LOGIN ${bracket(user)} WITH PASSWORD = '<strong password>';  -- if the login does not exist`,
            `USE ${bracket(db)};`,
            `CREATE USER ${bracket(user)} FOR LOGIN ${bracket(user)};`,
            `ALTER ROLE db_datareader ADD MEMBER ${bracket(user)};`,
            `GRANT SELECT ON SCHEMA::cdc TO ${bracket(user)};`,
          ].join("\n"),
        },
      },
      {
        key: "pk",
        title: "Tables without a primary key",
        sql: `SELECT s.name AS schema_name, t.name AS table_name
FROM sys.tables t
JOIN sys.schemas s ON s.schema_id = t.schema_id
WHERE t.is_ms_shipped = 0
  AND s.name <> 'cdc'
  AND OBJECTPROPERTY(t.object_id, 'TableHasPrimaryKey') = 0
ORDER BY 1, 2;`,
        ready: [
          "No captured table listed",
          "The target sink matches rows by key. Add a primary key to listed tables before Start.",
        ],
      },
    ],
  }
}

// ---------------------------------------------------------------- Oracle

const oracleGuide = (ctx: ReadinessContext): ReadinessGuide => {
  const params: ReadinessParam[] = [
    {
      name: "Log mode",
      values: { small: "ARCHIVELOG", medium: "ARCHIVELOG", large: "ARCHIVELOG" },
      apply: "Mount (outage)",
      disruptive: true,
      info: "LogMiner reads archived redo, so the database must run in ARCHIVELOG mode. Switching needs SHUTDOWN / STARTUP MOUNT.",
    },
    {
      name: "Supplemental logging (database)",
      values: { small: "MIN", medium: "MIN", large: "MIN" },
      apply: "Online",
      info: "ALTER DATABASE ADD SUPPLEMENTAL LOG DATA. Without it redo lacks the row identity LogMiner needs.",
    },
    {
      name: "Supplemental logging (table)",
      values: { small: "ALL COLUMNS", medium: "ALL COLUMNS", large: "ALL COLUMNS" },
      apply: "Per table",
      info: "ALTER TABLE ... ADD SUPPLEMENTAL LOG DATA (ALL) COLUMNS on every captured table, so updates carry full rows for the sink upsert.",
    },
    {
      name: "Redo log size",
      values: { small: "512M", medium: "2G", large: "4G" },
      apply: "Online",
      info: "Small redo logs switch often, which makes LogMiner sessions churn and lag. Aim for no more than ~4–6 log switches per hour at peak.",
    },
    {
      name: "Redo log groups",
      values: { small: "3", medium: "4", large: "6" },
      apply: "Online",
      info: "More groups give the archiver time to keep up under bursts, so the database never waits on 'checkpoint not complete'.",
    },
    {
      name: "db_recovery_file_dest_size",
      values: { small: "20G", medium: "100G", large: "500G" },
      apply: "Online",
      info: "Space for archived logs. When it fills, the database stops accepting writes. Size it for the retention below at peak redo rate.",
    },
    {
      name: "Archive log retention",
      values: { small: "2 days", medium: "3 days", large: "7 days" },
      apply: "RMAN schedule",
      info: "Archived logs must stay until the connector has mined them. If a needed log is deleted, the pipeline must re-snapshot.",
    },
  ]
  const user = (ctx.username || "c##dbzuser").toUpperCase()
  const owners = schemasOf(ctx.tables).map((s) => s.toUpperCase())
  const ownerList = (owners.length ? owners : ["<SCHEMA>"]).map(sqlString).join(", ")
  const tables = ctx.tables.length ? ctx.tables : ["APP.CUSTOMERS", "APP.ORDERS"]
  const suppTables = tables.map((t) => {
    const [s, name] = parts(t)
    return `ALTER TABLE ${s ? `${dq(s)}.` : ""}${dq(name)} ADD SUPPLEMENTAL LOG DATA (ALL) COLUMNS;`
  })
  const days = (tier: ReadinessTier) => pick(params, "Archive log retention", tier).split(" ")[0]

  return {
    title: "Oracle CDC readiness",
    summary: "LogMiner prerequisites with redo and archive sizing, and the checks to run on the source before Start.",
    tierHelp: LOG_TIER_HELP,
    params,
    scriptLabel: "Setup SQL",
    script: (tier) => {
      const groups = Number(pick(params, "Redo log groups", tier))
      const size = pick(params, "Redo log size", tier)
      return [
        "-- Run as SYSDBA (in CDB$ROOT for a multitenant database)",
        "-- 1. ARCHIVELOG — needs a short outage",
        "SHUTDOWN IMMEDIATE;",
        "STARTUP MOUNT;",
        "ALTER DATABASE ARCHIVELOG;",
        "ALTER DATABASE OPEN;",
        "",
        "-- 2. Archive destination (size first, then location)",
        `ALTER SYSTEM SET db_recovery_file_dest_size = ${pick(params, "db_recovery_file_dest_size", tier)} SCOPE = BOTH;`,
        "ALTER SYSTEM SET db_recovery_file_dest = '/u01/app/oracle/fast_recovery_area' SCOPE = BOTH;",
        "",
        "-- 3. Minimal supplemental logging",
        "ALTER DATABASE ADD SUPPLEMENTAL LOG DATA;",
        "",
        "-- 4. Redo logs: add new groups, then drop the old small ones once INACTIVE",
        ...Array.from({ length: groups }, (_, i) => `ALTER DATABASE ADD LOGFILE GROUP ${11 + i} SIZE ${size};`),
        "-- ALTER SYSTEM SWITCH LOGFILE;  ALTER SYSTEM CHECKPOINT;  ALTER DATABASE DROP LOGFILE GROUP <old>;",
        "",
        "-- 5. Table supplemental logging (as the table owner or DBA)",
        ...suppTables,
        "",
        `-- 6. Archive retention (RMAN, scheduled): DELETE NOPROMPT ARCHIVELOG ALL COMPLETED BEFORE 'SYSDATE-${days(tier)}';`,
      ].join("\n")
    },
    notes: () => [
      "Debezium mines redo with LogMiner, which adds CPU and PGA load on the source. On busy databases consider mining a standby.",
      "For a multitenant database the CDC user is a common user (C##...) created in CDB$ROOT with CONTAINER=ALL.",
      "Amazon RDS for Oracle uses rdsadmin.rdsadmin_util procedures (alter_supplemental_logging, set_configuration 'archivelog retention hours') instead of ALTER DATABASE.",
    ],
    checks: [
      {
        key: "database",
        title: "Archive log and supplemental logging",
        sql: `SELECT log_mode, supplemental_log_data_min, force_logging
FROM v$database;`,
        ready: ["log_mode = ARCHIVELOG", "supplemental_log_data_min = YES or IMPLICIT"],
      },
      {
        key: "redo",
        title: "Redo and archive capacity",
        sql: `SELECT group#, thread#, bytes / 1024 / 1024 AS size_mb, members, status
FROM v$log
ORDER BY group#;

SELECT name,
       ROUND(space_limit / 1024 / 1024 / 1024) AS limit_gb,
       ROUND(space_used / 1024 / 1024 / 1024) AS used_gb
FROM v$recovery_file_dest;

SELECT TO_CHAR(first_time, 'YYYY-MM-DD HH24') AS hour, COUNT(*) AS switches
FROM v$log_history
WHERE first_time > SYSDATE - 1
GROUP BY TO_CHAR(first_time, 'YYYY-MM-DD HH24')
ORDER BY 1;`,
        ready: [
          "Redo groups and sizes at least the tier above",
          "No more than ~4–6 log switches in your busiest hour",
          "used_gb well below limit_gb",
        ],
      },
      {
        key: "tables",
        title: "Table supplemental logging",
        sql: `SELECT owner, table_name, log_group_type
FROM all_log_groups
WHERE owner IN (${ownerList})
ORDER BY 1, 2;`,
        ready: ["ALL COLUMN LOGGING for every table this pipeline captures"],
        extra: { label: "Enable for the selected tables", sql: suppTables.join("\n") },
      },
      {
        key: "user",
        title: "LogMiner user",
        sql: `SELECT 'SYSTEM' AS kind, privilege AS name FROM dba_sys_privs WHERE grantee = ${sqlString(user)}
UNION ALL
SELECT 'ROLE', granted_role FROM dba_role_privs WHERE grantee = ${sqlString(user)}
UNION ALL
SELECT 'OBJECT', privilege || ' ON ' || owner || '.' || table_name FROM dba_tab_privs WHERE grantee = ${sqlString(user)}
ORDER BY 1, 2;`,
        ready: [
          "CREATE SESSION, LOGMINING, SELECT ANY TRANSACTION, FLASHBACK ANY TABLE, SELECT ANY TABLE",
          "SELECT_CATALOG_ROLE and EXECUTE_CATALOG_ROLE, plus EXECUTE on DBMS_LOGMNR",
          "CREATE TABLE and a tablespace quota (Debezium keeps a small flush table)",
          "SET CONTAINER on a multitenant database",
        ],
        extra: {
          label: "Grant if missing",
          sql: [
            `-- Multitenant: run in CDB$ROOT and add CONTAINER=ALL to each GRANT`,
            `CREATE USER ${user} IDENTIFIED BY "<strong password>" DEFAULT TABLESPACE users QUOTA UNLIMITED ON users;`,
            `GRANT CREATE SESSION, SET CONTAINER, LOGMINING, SELECT ANY TRANSACTION TO ${user};`,
            `GRANT FLASHBACK ANY TABLE, SELECT ANY TABLE, LOCK ANY TABLE TO ${user};`,
            `GRANT CREATE TABLE, CREATE SEQUENCE TO ${user};`,
            `GRANT SELECT_CATALOG_ROLE, EXECUTE_CATALOG_ROLE TO ${user};`,
            `GRANT EXECUTE ON DBMS_LOGMNR TO ${user};`,
            `GRANT EXECUTE ON DBMS_LOGMNR_D TO ${user};`,
            ...[
              "V_$DATABASE",
              "V_$LOG",
              "V_$LOG_HISTORY",
              "V_$LOGFILE",
              "V_$ARCHIVED_LOG",
              "V_$ARCHIVE_DEST_STATUS",
              "V_$LOGMNR_LOGS",
              "V_$LOGMNR_CONTENTS",
              "V_$LOGMNR_PARAMETERS",
              "V_$TRANSACTION",
              "V_$MYSTAT",
              "V_$STATNAME",
            ].map((view) => `GRANT SELECT ON ${view} TO ${user};`),
          ].join("\n"),
        },
      },
      {
        key: "pk",
        title: "Tables without a primary key",
        sql: `SELECT t.owner, t.table_name
FROM all_tables t
WHERE t.owner IN (${ownerList})
  AND NOT EXISTS (
      SELECT 1 FROM all_constraints c
      WHERE c.owner = t.owner
        AND c.table_name = t.table_name
        AND c.constraint_type = 'P'
  )
ORDER BY 1, 2;`,
        ready: [
          "No captured table listed",
          "The target sink matches rows by key. Add a primary key to listed tables before Start.",
        ],
      },
    ],
  }
}

// ---------------------------------------------------------------- DB2 (trigger)

const db2Guide = (ctx: ReadinessContext): ReadinessGuide => {
  const params: ReadinessParam[] = [
    {
      name: "Db2 version",
      values: { small: "11.1.4.4+", medium: "11.5+", large: "11.5+" },
      apply: "Upgrade",
      info: "Capture triggers build each row as JSON with JSON_OBJECT(KEY … VALUE …), available from Db2 11.1.4.4 (LUW).",
    },
    {
      name: "LOGFILSIZ (4 KB pages)",
      values: { small: "10240", medium: "65536", large: "262144" },
      apply: "Reactivate DB",
      disruptive: true,
      info: "Size of each transaction log file. Triggers add one changelog insert per row change, so log volume on captured tables roughly doubles.",
    },
    {
      name: "LOGPRIMARY",
      values: { small: "13", medium: "20", large: "40" },
      apply: "Reactivate DB",
      disruptive: true,
      info: "Pre-allocated log files. Long transactions on captured tables need more active log space than before.",
    },
    {
      name: "LOGSECOND",
      values: { small: "12", medium: "40", large: "80" },
      apply: "Online",
      info: "Extra log files allocated on demand when primary logs fill. Protects against SQL0964C 'transaction log full'.",
    },
    ...changelogParams(),
  ]
  const db = ctx.database || "<database>"
  const user = (ctx.username || "<user>").toUpperCase()
  const tables = ctx.tables.length ? ctx.tables : ["APP.CUSTOMERS", "APP.ORDERS"]
  const hours = (tier: ReadinessTier) => pick(params, "Changelog retention (hours)", tier)

  return {
    title: "IBM DB2 capture readiness",
    summary: "Trigger capture prerequisites with log sizing, and the checks to run on the source before Start.",
    modeNote: triggerModeNote("IBM DB2", ctx.captureMode),
    tierHelp: TRIGGER_TIER_HELP,
    params,
    scriptLabel: "Setup",
    script: (tier) =>
      [
        "-- db2 CLP as instance owner or DBADM",
        `UPDATE DB CFG FOR ${db} USING LOGFILSIZ ${pick(params, "LOGFILSIZ (4 KB pages)", tier)} LOGPRIMARY ${pick(params, "LOGPRIMARY", tier)} LOGSECOND ${pick(params, "LOGSECOND", tier)};`,
        `-- LOGFILSIZ and LOGPRIMARY apply after: DEACTIVATE DB ${db};  ACTIVATE DB ${db};`,
        "",
        "-- Hyper CDC does not prune the changelog on DB2. Schedule this (ADMIN_TASK_ADD or cron):",
        `DELETE FROM hyper_cdc.change_log WHERE changed_at < CURRENT TIMESTAMP - ${hours(tier)} HOURS;`,
      ].join("\n"),
    notes: () => [
      "Install creates schema HYPER_CDC, table HYPER_CDC.CHANGE_LOG and three HYPER_CDC_* triggers per captured table.",
      "A trigger failure fails the application's own INSERT/UPDATE/DELETE. Test the install on a non-production copy first.",
    ],
    checks: [
      {
        key: "version",
        title: "Version and log configuration",
        sql: `SELECT service_level FROM SYSIBMADM.ENV_INST_INFO;

SELECT name, value
FROM SYSIBMADM.DBCFG
WHERE name IN ('logfilsiz', 'logprimary', 'logsecond', 'logarchmeth1');`,
        ready: ["service_level is v11.1.4.4 or later", "Log values at least the tier above"],
      },
      {
        key: "user",
        title: "Install privileges",
        sql: `SELECT grantee, dbadmauth, createtabauth, implschemaauth
FROM SYSCAT.DBAUTH
WHERE grantee = ${sqlString(user)};

SELECT tabschema, tabname, alterauth, selectauth, controlauth
FROM SYSCAT.TABAUTH
WHERE grantee = ${sqlString(user)};`,
        ready: [
          "DBADMAUTH = Y, or",
          "CREATETABAUTH = Y and IMPLSCHEMAAUTH = Y (or a DBA pre-creates HYPER_CDC and grants CREATEIN)",
          "ALTERAUTH = Y (or CONTROLAUTH) and SELECTAUTH = Y on each captured table",
        ],
        extra: {
          label: "Grant if missing",
          sql: [
            `GRANT CREATETAB, IMPLICIT_SCHEMA ON DATABASE TO USER ${user};`,
            ...tables.map((t) => `GRANT ALTER, SELECT ON TABLE ${t} TO USER ${user};`),
          ].join("\n"),
        },
      },
      {
        key: "installed",
        title: "Changelog and triggers",
        sql: `SELECT tabschema, tabname
FROM SYSCAT.TABLES
WHERE tabschema = 'HYPER_CDC';

SELECT trigschema, trigname, tabschema, tabname, valid
FROM SYSCAT.TRIGGERS
WHERE UPPER(trigname) LIKE 'HYPER_CDC%'
ORDER BY tabschema, tabname;`,
        ready: [
          "Before install: no rows. After install: CHANGE_LOG plus three triggers per captured table",
          "valid = Y on every trigger",
        ],
      },
      {
        key: "backlog",
        title: "Changelog backlog",
        sql: `SELECT COUNT(*) AS pending_rows, MIN(changed_at) AS oldest
FROM hyper_cdc.change_log;`,
        ready: ["oldest is within your retention window (otherwise schedule the purge above)"],
      },
    ],
  }
}

// ---------------------------------------------------------------- Informix (trigger)

const informixGuide = (ctx: ReadinessContext): ReadinessGuide => {
  const params: ReadinessParam[] = [
    {
      name: "Logical log size (KB)",
      values: { small: "10000", medium: "102400", large: "512000" },
      apply: "Add new logs",
      info: "Size of each logical log (onconfig LOGSIZE for new logs). Triggers add one changelog insert per row change, so log volume grows.",
    },
    {
      name: "Logical logs",
      values: { small: "6", medium: "10", large: "20" },
      apply: "Add new logs",
      info: "Number of logical logs. Too few, with long transactions on captured tables, risks hitting the long-transaction high-water mark (LTXHWM).",
    },
    ...changelogParams(),
  ]
  const user = ctx.username || "<user>"
  const hours = (tier: ReadinessTier) => pick(params, "Changelog retention (hours)", tier)

  return {
    title: "Informix capture readiness",
    summary: "Trigger capture prerequisites with logical-log sizing, and the checks to run on the source before Start.",
    modeNote: triggerModeNote("Informix", ctx.captureMode),
    tierHelp: TRIGGER_TIER_HELP,
    params,
    scriptLabel: "Setup",
    script: (tier) =>
      [
        "-- As user informix: add logical logs of the new size, then switch and drop the old ones",
        `--   onparams -a -d <dbspace> -s ${pick(params, "Logical log size (KB)", tier)}    (repeat for ${pick(params, "Logical logs", tier)} logs)`,
        "--   onmode -l                               (move to a new log)",
        "--   onparams -d -l <old log number> -y      (once the old log is backed up and free)",
        "",
        "-- Hyper CDC does not prune the changelog on Informix. Schedule this (sysadmin ph_task or cron):",
        `DELETE FROM hyper_cdc_change_log WHERE changed_at < CURRENT YEAR TO FRACTION(5) - ${hours(tier)} UNITS HOUR;`,
      ].join("\n"),
    notes: () => [
      "Install creates table hyper_cdc_change_log and three hyper_cdc_* triggers per captured table.",
      "pk and row_data are LVARCHAR(32739). A row whose JSON is larger fails the trigger, and with it the application's write. Check very wide tables first.",
      "Only the table owner or a DBA can create triggers on a table.",
    ],
    checks: [
      {
        key: "logs",
        title: "Logical logs",
        sql: `SELECT number, size, used, is_used, is_current, is_backed_up
FROM sysmaster:syslogs
ORDER BY number;`,
        ready: ["Log count and size at least the tier above", "Logs are being backed up (is_backed_up = 1) so they can be reused"],
      },
      {
        key: "user",
        title: "Install privileges",
        sql: `SELECT username, usertype
FROM sysusers
WHERE username = ${sqlString(user)};`,
        ready: ["usertype = D (DBA), or R (Resource) and owner of every captured table"],
        extra: { label: "Grant if missing", sql: `GRANT RESOURCE TO ${dq(user)};\n-- or, when the user does not own the tables:\nGRANT DBA TO ${dq(user)};` },
      },
      {
        key: "installed",
        title: "Changelog and triggers",
        sql: `SELECT tabname, nrows
FROM systables
WHERE tabname = 'hyper_cdc_change_log';

SELECT g.trigname, t.tabname
FROM systriggers g
JOIN systables t ON t.tabid = g.tabid
WHERE g.trigname LIKE 'hyper_cdc%'
ORDER BY t.tabname;`,
        ready: ["Before install: no rows. After install: the changelog plus three triggers per captured table"],
      },
      {
        key: "backlog",
        title: "Changelog backlog",
        sql: `SELECT COUNT(*) AS pending_rows, MIN(changed_at) AS oldest
FROM hyper_cdc_change_log;`,
        ready: ["oldest is within your retention window (otherwise schedule the purge above)"],
      },
    ],
  }
}

// ---------------------------------------------------------------- SQLite (trigger)

const sqliteGuide = (ctx: ReadinessContext): ReadinessGuide => {
  const params: ReadinessParam[] = [
    {
      name: "journal_mode",
      values: { small: "WAL", medium: "WAL", large: "WAL" },
      apply: "Persistent",
      info: "WAL lets the changelog poller read while applications write. In the default rollback-journal mode readers and writers block each other.",
    },
    {
      name: "busy_timeout (ms)",
      values: { small: "5000", medium: "10000", large: "30000" },
      apply: "Per connection",
      info: "How long a writer waits for a lock instead of failing with SQLITE_BUSY. Set it in every application that writes this file.",
    },
    {
      name: "wal_autocheckpoint (pages)",
      values: { small: "1000", medium: "1000", large: "10000" },
      apply: "Per connection",
      info: "WAL size before an automatic checkpoint. Larger values mean fewer checkpoint stalls under heavy writes, at the cost of a bigger -wal file.",
    },
    ...changelogParams(),
  ]

  return {
    title: "SQLite capture readiness",
    summary: "Trigger capture prerequisites and WAL settings, and the checks to run on the database file before Start.",
    modeNote: triggerModeNote("SQLite", ctx.captureMode),
    tierHelp: TRIGGER_TIER_HELP,
    params,
    scriptLabel: "PRAGMAs",
    script: (tier) =>
      [
        "PRAGMA journal_mode = WAL;  -- persistent for the database file",
        `PRAGMA wal_autocheckpoint = ${pick(params, "wal_autocheckpoint (pages)", tier)};`,
        `-- Applications writing this file should also set: PRAGMA busy_timeout = ${pick(params, "busy_timeout (ms)", tier)};`,
      ].join("\n"),
    notes: () => [
      "Install creates table hyper_cdc_change_log and hyper_cdc_* triggers. Hyper CDC prunes the changelog itself using the retention above.",
      "Triggers use json_object(), built in from SQLite 3.38 (earlier builds need the JSON1 extension).",
      "The orchestrator and Kafka Connect must both be able to open the database file. Keep it on a local disk, not a network share.",
    ],
    checks: [
      {
        key: "settings",
        title: "Version and journal mode",
        sql: `SELECT sqlite_version();
PRAGMA journal_mode;
SELECT json_object('ok', 1);`,
        ready: ["journal_mode = wal", "json_object returns {\"ok\":1} (no 'no such function' error)"],
      },
      {
        key: "installed",
        title: "Changelog and triggers",
        sql: `SELECT name, tbl_name
FROM sqlite_master
WHERE type = 'trigger' AND name LIKE 'hyper_cdc%'
ORDER BY tbl_name;

SELECT COUNT(*) AS pending_rows, MIN(changed_at) AS oldest
FROM hyper_cdc_change_log;`,
        ready: ["After install: triggers for every captured table", "oldest is within the retention window"],
      },
      {
        key: "pk",
        title: "Tables without a primary key",
        sql: `SELECT m.name
FROM sqlite_master m
WHERE m.type = 'table'
  AND m.name NOT LIKE 'sqlite_%'
  AND m.name NOT LIKE 'hyper_cdc%'
  AND NOT EXISTS (SELECT 1 FROM pragma_table_info(m.name) p WHERE p.pk > 0)
ORDER BY 1;`,
        ready: [
          "No captured table listed",
          "Target apply matches rows by the target table's primary key (or the mapping's key columns). Without one, updates and deletes cannot be applied.",
        ],
      },
    ],
  }
}

/** Readiness guide for a source type and capture mode, or null when none applies. */
export const readinessGuide = (sourceType: string, ctx: ReadinessContext): ReadinessGuide | null => {
  const type = sourceType.trim().toLowerCase()
  if (LOG_CDC_SOURCES.includes(type)) {
    if (ctx.captureMode !== "logical") return null
    if (type === "postgres") return postgresGuide(ctx)
    if (type === "mysql") return mysqlFamilyGuide(ctx, false)
    if (type === "mariadb") return mysqlFamilyGuide(ctx, true)
    if (type === "sqlserver") return sqlServerGuide(ctx)
    return oracleGuide(ctx)
  }
  if (TRIGGER_ONLY_SOURCES.includes(type)) {
    if (ctx.captureMode !== "logical" && ctx.captureMode !== "trigger") return null
    if (type === "db2") return db2Guide(ctx)
    if (type === "informix") return informixGuide(ctx)
    return sqliteGuide(ctx)
  }
  return null
}
