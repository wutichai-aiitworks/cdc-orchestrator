/**
 * Target CREATE TABLE script from an inspected source table.
 *
 * Keeps the source primary key, unique constraints and indexes, and writes the
 * syntax of the *target* database (quoting, "only if missing" guards, types).
 *
 * Rules that match the CDC runtime:
 * - Oracle → Postgres folds every identifier to lowercase, like the copy, sink and
 *   table status (`fold_oracle_postgres_target` in the orchestrator).
 * - Identity / serial / sequence defaults are dropped: the sink writes source ids.
 * - Foreign keys are never emitted: an initial load fills tables in no fixed order.
 * - Anything that cannot be converted safely is listed as a `--` note, never guessed.
 *
 * `statements` run one per call (Oracle/SQL Server JDBC run one statement at a
 * time). They carry no trailing `;` except the PL/SQL `END;` that Oracle needs.
 */
import { emitType, normalizeType } from "./dbTypeTranslator"

export type DdlDialect = "postgres" | "oracle" | "sqlserver" | "mysql"

export interface DdlSourceColumn {
  name: string
  data_type: string
  nullable: boolean
  primary_key?: boolean
  pk_position?: number | null
  size?: number | null
  scale?: number | null
  default_value?: string | null
  auto_increment?: boolean
}

export interface DdlSourceIndex {
  name: string
  unique: boolean
  columns: string[]
  descending?: string[]
  filter?: string | null
  expression?: boolean
}

export interface TargetDdlInput {
  sourceDialect: string
  targetDialect: string
  targetSchema: string
  targetTable: string
  columns: DdlSourceColumn[]
  primaryKey?: string[]
  /** `undefined` = the inspector did not report indexes (older cdc-jdbc-admin). */
  indexes?: DdlSourceIndex[]
  /** Emit CREATE SCHEMA when the target has schemas (Postgres, SQL Server). Default true. */
  createSchema?: boolean
}

export interface TargetDdlResult {
  dialect: DdlDialect | null
  statements: string[]
  /** Copy/paste script: header, notes, statements with the dialect's terminators. */
  script: string
  notes: string[]
}

export function ddlDialect(type: string): DdlDialect | null {
  switch (type.trim().toLowerCase()) {
    case "postgres":
    case "postgresql":
      return "postgres"
    case "oracle":
      return "oracle"
    case "sqlserver":
    case "mssql":
      return "sqlserver"
    case "mysql":
    case "mariadb":
      return "mysql"
    default:
      return null
  }
}

// ─── Identifiers ──────────────────────────────────────────────────────────────

const MAX_IDENT: Record<DdlDialect, number> = { postgres: 63, oracle: 128, sqlserver: 128, mysql: 64 }

function quoteIdent(name: string, d: DdlDialect): string {
  switch (d) {
    case "sqlserver":
      return `[${name.replace(/]/g, "]]")}]`
    case "mysql":
      return `\`${name.replace(/`/g, "``")}\``
    default:
      return `"${name.replace(/"/g, '""')}"`
  }
}

function sqlString(value: string): string {
  return `'${value.replace(/'/g, "''")}'`
}

// ─── Types ────────────────────────────────────────────────────────────────────

/** Types whose JDBC COLUMN_SIZE is a character/byte length. */
const LENGTH_TYPES = /^(N?VARCHAR2?|N?CHAR|BPCHAR|CHARACTER( VARYING)?|VARBINARY|BINARY|RAW|UROWID)$/
/** Types whose COLUMN_SIZE/DECIMAL_DIGITS are precision/scale. */
const PRECISION_TYPES = /^(NUMBER|NUMERIC|DECIMAL|DEC)$/
/** Sizes above this mean "unbounded" (Postgres varchar/numeric without typmod, nvarchar(max)). */
const UNBOUNDED_SIZE = 10_485_760

/**
 * Rebuild the declared type when the inspector returned a bare TYPE_NAME plus
 * COLUMN_SIZE/DECIMAL_DIGITS (jdbc-admin). Native inspect already includes `(n)`.
 */
export function declaredSourceType(col: DdlSourceColumn): string {
  const raw = col.data_type.trim()
  if (raw.includes("(")) return raw
  const base = raw.toUpperCase().replace(/\s+IDENTITY$/, "")
  const size = col.size ?? undefined
  const scale = col.scale ?? undefined
  if (LENGTH_TYPES.test(base) && size && size > 0 && size < UNBOUNDED_SIZE) {
    return `${raw.replace(/\s+identity$/i, "")}(${size})`
  }
  if (PRECISION_TYPES.test(base) && size && size > 0 && size <= 1000) {
    // Oracle bare NUMBER reports scale -127; treat as unconstrained scale.
    if (scale !== undefined && scale > 0) return `${raw}(${size},${scale})`
    return `${raw}(${size})`
  }
  return raw.replace(/\s+identity$/i, "")
}

const PG_SERIAL: Record<string, string> = {
  SMALLSERIAL: "SMALLINT",
  SERIAL2: "SMALLINT",
  SERIAL: "INTEGER",
  SERIAL4: "INTEGER",
  BIGSERIAL: "BIGINT",
  SERIAL8: "BIGINT",
}

function targetType(
  col: DdlSourceColumn,
  src: DdlDialect | null,
  tgt: DdlDialect,
  notes: string[],
): string {
  const declared = declaredSourceType(col)
  const upper = declared.toUpperCase()
  if (src === "postgres" && PG_SERIAL[upper]) {
    const asInt = PG_SERIAL[upper]
    return tgt === "postgres" ? asInt : emitType(normalizeType(asInt, "postgres"), tgt)
  }
  if (src === tgt) return declared
  if (!src) {
    notes.push(`Column ${col.name}: source database type unknown, kept "${declared}" as is`)
    return declared
  }
  const canonical = normalizeType(declared, src)
  if (canonical.kind === "unknown") {
    notes.push(`Column ${col.name}: no ${tgt} equivalent for ${src} type "${declared}", kept as is — review before running`)
    return declared
  }
  if (tgt === "oracle" && canonical.kind === "varchar" && canonical.length !== undefined) {
    // Character semantics: a Postgres varchar(50) holds 50 characters, not bytes.
    return canonical.length > 4000 ? "CLOB" : `VARCHAR2(${canonical.length} CHAR)`
  }
  return emitType(canonical, tgt)
}

// ─── Defaults ─────────────────────────────────────────────────────────────────

const NOW_RE = /^(now\(\)|current_timestamp(\(\d*\))?|sysdate|systimestamp|getdate\(\)|sysdatetime\(\)|localtimestamp)$/i

/** Translate a source default, or return null (with a note) when it is not portable. */
export function targetDefault(
  col: DdlSourceColumn,
  tgt: DdlDialect,
  notes: string[],
): string | null {
  let expr = (col.default_value ?? "").trim()
  if (!expr || /^null$/i.test(expr)) return null
  if (col.auto_increment || /^nextval\(/i.test(expr) || /\.nextval$/i.test(expr)) return null
  // SQL Server wraps defaults in parentheses: ((0)), ('abc'), (getdate()).
  while (/^\(.*\)$/.test(expr) && balanced(expr.slice(1, -1))) expr = expr.slice(1, -1).trim()
  // Postgres casts: 'abc'::character varying, 0::numeric.
  expr = expr.replace(/::[a-z_ ]+(\(\d+(,\s*\d+)?\))?(\[\])?$/i, "").trim()

  if (/^-?\d+(\.\d+)?$/.test(expr)) return expr
  if (/^N?'(?:[^']|'')*'$/.test(expr)) return expr.replace(/^N/, "")
  if (NOW_RE.test(expr)) return "CURRENT_TIMESTAMP"
  if (/^(current_date|trunc\(sysdate\))$/i.test(expr)) {
    return tgt === "sqlserver" ? "CAST(GETDATE() AS date)" : "CURRENT_DATE"
  }
  if (/^(true|false)$/i.test(expr)) {
    const on = /^true$/i.test(expr)
    return tgt === "postgres" || tgt === "mysql" ? (on ? "TRUE" : "FALSE") : on ? "1" : "0"
  }
  notes.push(`Column ${col.name}: default ${expr} not converted — add it by hand if needed`)
  return null
}

function balanced(s: string): boolean {
  let depth = 0
  for (const c of s) {
    if (c === "(") depth++
    else if (c === ")" && --depth < 0) return false
  }
  return depth === 0
}

// ─── Generator ────────────────────────────────────────────────────────────────

export function generateTargetDdl(input: TargetDdlInput): TargetDdlResult {
  const tgt = ddlDialect(input.targetDialect)
  const src = ddlDialect(input.sourceDialect)
  const notes: string[] = []
  if (!tgt) {
    return {
      dialect: null,
      statements: [],
      script: `-- Target type "${input.targetDialect}" is not a SQL database supported by the script generator.`,
      notes: [`Unsupported target type ${input.targetDialect}`],
    }
  }
  const fold = src === "oracle" && tgt === "postgres"
  const ident = (n: string) => (fold ? n.toLowerCase() : n)
  const q = (n: string) => quoteIdent(ident(n), tgt)

  const schema = input.targetSchema.trim()
  const table = input.targetTable.trim()
  const qualified = schema ? `${q(schema)}.${q(table)}` : q(table)
  const columns = input.columns
  const columnNames = new Set(columns.map((c) => c.name.toLowerCase()))

  const pk = (input.primaryKey?.length
    ? input.primaryKey
    : columns
        .filter((c) => c.primary_key)
        .sort((a, b) => (a.pk_position ?? 0) - (b.pk_position ?? 0))
        .map((c) => c.name)
  ).filter((c) => columnNames.has(c.toLowerCase()))
  const pkSet = new Set(pk.map((c) => c.toLowerCase()))

  // ── Columns ──
  const columnLines = columns.map((col) => {
    const type = targetType(col, src, tgt, notes)
    const def = targetDefault(col, tgt, notes)
    const notNull = !col.nullable || pkSet.has(col.name.toLowerCase())
    return `  ${q(col.name)} ${type}${def !== null ? ` DEFAULT ${def}` : ""}${notNull ? " NOT NULL" : ""}`
  })
  const identityCols = columns.filter((c) => c.auto_increment || /^nextval\(/i.test(c.default_value ?? ""))
  if (identityCols.length) {
    notes.push(
      `Identity/sequence dropped on ${identityCols.map((c) => c.name).join(", ")} — the pipeline writes the source values`,
    )
  }
  if (pk.length) columnLines.push(`  PRIMARY KEY (${pk.map(q).join(", ")})`)
  else notes.push("Source table has no primary key — set a Logical PK in the mapping so the sink can upsert")

  // ── Indexes ──
  const pkKey = pk.map((c) => c.toLowerCase()).join(",")
  const seen = new Set<string>()
  const indexes: DdlSourceIndex[] = []
  if (input.indexes === undefined) {
    notes.push("Index details were not reported by the source inspector (update cdc-jdbc-admin) — only the primary key is included")
  }
  for (const ix of input.indexes ?? []) {
    const cols = ix.columns ?? []
    const key = cols.map((c) => c.toLowerCase()).join(",")
    if (ix.unique && cols.length && key === pkKey) continue // the primary key's own index
    if (ix.expression || !cols.length || cols.some((c) => !columnNames.has(c.toLowerCase()))) {
      notes.push(`Index ${ix.name} skipped: expression/function-based keys need manual conversion`)
      continue
    }
    if (ix.filter) {
      notes.push(`Index ${ix.name} skipped: partial index (WHERE ${ix.filter}) needs manual conversion`)
      continue
    }
    const dedupe = `${ix.unique ? "u" : "i"}:${key}`
    if (seen.has(dedupe)) continue
    seen.add(dedupe)
    indexes.push(ix)
  }

  const indexName = (ix: DdlSourceIndex) => ident(ix.name).slice(0, MAX_IDENT[tgt])
  const indexCols = (ix: DdlSourceIndex) => {
    const desc = new Set((ix.descending ?? []).map((c) => c.toLowerCase()))
    return ix.columns.map((c) => `${q(c)}${desc.has(c.toLowerCase()) ? " DESC" : ""}`).join(", ")
  }

  if (tgt === "mysql") {
    for (const ix of indexes) {
      columnLines.push(`  ${ix.unique ? "UNIQUE KEY" : "KEY"} ${quoteIdent(indexName(ix), tgt)} (${indexCols(ix)})`)
    }
    const blobKeys = columns.filter(
      (c) =>
        (pkSet.has(c.name.toLowerCase()) || indexes.some((ix) => ix.columns.some((k) => k.toLowerCase() === c.name.toLowerCase()))) &&
        /TEXT|BLOB/.test(targetType(c, src, tgt, [])),
    )
    if (blobKeys.length) {
      notes.push(`MySQL needs a prefix length to index TEXT/BLOB columns: ${blobKeys.map((c) => c.name).join(", ")}`)
    }
  }

  const body = `(\n${columnLines.join(",\n")}\n)`
  const statements: string[] = []

  switch (tgt) {
    case "postgres": {
      if (schema && input.createSchema !== false) statements.push(`CREATE SCHEMA IF NOT EXISTS ${q(schema)}`)
      statements.push(`CREATE TABLE IF NOT EXISTS ${qualified} ${body}`)
      for (const ix of indexes) {
        statements.push(
          `CREATE ${ix.unique ? "UNIQUE " : ""}INDEX IF NOT EXISTS ${quoteIdent(indexName(ix), tgt)} ON ${qualified} (${indexCols(ix)})`,
        )
      }
      break
    }
    case "sqlserver": {
      const objectRef = sqlString(qualified)
      if (schema && input.createSchema !== false) {
        statements.push(
          `IF SCHEMA_ID(${sqlString(ident(schema))}) IS NULL EXEC(N${sqlString(`CREATE SCHEMA ${q(schema)}`)})`,
        )
      }
      statements.push(`IF OBJECT_ID(N${objectRef}, N'U') IS NULL\nCREATE TABLE ${qualified} ${body}`)
      const nullableUnique = indexes.filter(
        (ix) => ix.unique && ix.columns.some((c) => columns.find((k) => k.name.toLowerCase() === c.toLowerCase())?.nullable),
      )
      for (const ix of indexes) {
        const name = indexName(ix)
        statements.push(
          `IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = N${sqlString(name)} AND object_id = OBJECT_ID(N${objectRef}))\n` +
            `CREATE ${ix.unique ? "UNIQUE " : ""}INDEX ${quoteIdent(name, tgt)} ON ${qualified} (${indexCols(ix)})`,
        )
      }
      if (nullableUnique.length) {
        notes.push(
          `SQL Server unique indexes allow only one NULL (${nullableUnique.map((ix) => ix.name).join(", ")}); add WHERE col IS NOT NULL if the source has several`,
        )
      }
      break
    }
    case "mysql": {
      statements.push(`CREATE TABLE IF NOT EXISTS ${qualified} ${body}`)
      break
    }
    case "oracle": {
      // Oracle has no IF NOT EXISTS before 23ai: ignore "name already used" (ORA-00955)
      // and, for indexes, "column list already indexed" (ORA-01408).
      const guarded = (ddl: string, codes: number[]) =>
        `BEGIN\n  EXECUTE IMMEDIATE ${sqlString(ddl)};\nEXCEPTION\n  WHEN OTHERS THEN\n` +
        `    IF SQLCODE NOT IN (${codes.join(", ")}) THEN RAISE; END IF;\nEND;`
      statements.push(guarded(`CREATE TABLE ${qualified} ${body}`, [-955]))
      for (const ix of indexes) {
        statements.push(
          guarded(
            `CREATE ${ix.unique ? "UNIQUE " : ""}INDEX ${quoteIdent(indexName(ix), tgt)} ON ${qualified} (${indexCols(ix)})`,
            [-955, -1408],
          ),
        )
      }
      if (schema) notes.push(`Oracle schema ${ident(schema)} is a user and must already exist`)
      break
    }
  }

  const header = [
    `-- Target table for ${input.sourceDialect} → ${input.targetDialect}: ${schema ? `${ident(schema)}.` : ""}${ident(table)}`,
    `-- Primary key: ${pk.length ? pk.map(ident).join(", ") : "none"} · indexes: ${indexes.length}`,
    ...notes.map((n) => `-- NOTE: ${n}`),
  ]
  const terminated = statements.map((s) => (tgt === "oracle" ? `${s}\n/` : `${s};`))
  return { dialect: tgt, statements, script: `${header.join("\n")}\n\n${terminated.join("\n\n")}\n`, notes }
}
