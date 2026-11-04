# ADR 0003 — Generate target tables from the source structure (mapping step)

Status: Accepted
Owner: Wutichai
Last updated: 2026-11-01

## Context

In the pipeline wizard's mapping step, rows marked "Same name · new" or
"Renamed · new" point at target tables that don't exist yet. Operators asked for
a script that creates them with the source's primary key, unique constraints and
indexes, written in the **target** database's syntax.

Before this change the source inspector (`cdc-jdbc-admin`) returned only column
name, bare `TYPE_NAME`, nullability and the PK. Lengths and precision were lost
(`VARCHAR2`, not `VARCHAR2(50)`), there was no index metadata, and the column
`unique` flag was a copy of the PK flag. The Schema Compare page's generator
emitted Postgres syntax for every target, with no keys.

## Decision

1. **Metadata at the source.** jdbc-admin `/v1/inspect` adds `size`, `scale`,
   `default_value` and `auto_increment` per column, plus `indexes[]` (name,
   unique, key columns, descending columns, partial filter) from
   `DatabaseMetaData.getIndexInfo(..., approximate=true)`. `approximate=true` is
   required: with `false` the Oracle driver may run `ANALYZE TABLE` on the
   customer's source. Native Postgres inspect returns the same shape from
   `pg_index` (key columns only, `INCLUDE` columns excluded). `indexes` is absent,
   not empty, when an older jdbc-admin doesn't report it, so the UI can tell the
   two apart.
2. **One generator in the UI** (`utils/targetDdl.ts`, both trees). Same dialect
   keeps the declared source type; across dialects it goes through
   `dbTypeTranslator`. Targets: Postgres (`IF NOT EXISTS`), SQL Server
   (`OBJECT_ID` / `sys.indexes` guards), MySQL (keys inline so
   `CREATE TABLE IF NOT EXISTS` stays idempotent), Oracle (`EXECUTE IMMEDIATE`
   blocks that ignore ORA-00955 and ORA-01408). Every script is safe to re-run.
3. **Match the runtime, not the source verbatim.**
   - Oracle → Postgres lowercases table, column and index names, the same rule
     as `fold_oracle_postgres_target` (copy, sink and table status agree).
   - Identity, serial and `nextval` defaults are dropped: the sink writes source
     ids, and `GENERATED ALWAYS` would reject them.
   - Foreign keys are never emitted: initial loads fill tables in no fixed order.
   - Expression and partial indexes, and defaults that call functions without a
     clear equivalent, become `-- NOTE` lines. The generator never guesses.
   - A Postgres `varchar(n)` going to Oracle becomes `VARCHAR2(n CHAR)`
     (characters, not bytes).
4. **A narrow execution endpoint.** `POST /cdc/profiles/catalog/target/ddl` takes
   `{target, statements[], profile_id?}`. It merges saved credentials like the
   other wizard endpoints, runs statements one at a time (Oracle and SQL Server
   JDBC run one statement per call), stops at the first failure, and writes
   `target.create_tables` to the audit trail. A guard accepts only
   CREATE SCHEMA/TABLE/INDEX, optionally behind an `IF` guard or inside a
   `BEGIN … END;` block whose string literals are checked recursively. Quoted
   identifiers and literals are blanked first, so a column named `"delete"`
   passes and a chained `; DROP` doesn't. `compare/target/exec-ddl` stays as the
   general-purpose DDL path.

## Rejected

- **Generating DDL in Rust.** The UI already owned the type translator and
  needed a live preview. One TypeScript module with unit tests (`node --test`,
  no framework) keeps a single source of truth.
- **Sending the whole script as one statement.** Works on Postgres, fails on
  Oracle and SQL Server JDBC.
- **Reusing `exec-ddl` from the wizard.** It has no `profile_id` credential
  merge (the wizard holds a masked password) and no create-only guard.

## Consequences

- Size, scale, default and index details need a rebuilt `cdc-jdbc-admin`
  image. Until then the dialog still works, says that index details are
  missing, and includes only the primary key.
- Oracle → Postgres output is covered by unit tests. The only Oracle in reach is
  a customer production system and is used read-only.
