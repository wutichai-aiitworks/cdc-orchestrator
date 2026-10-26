/**
 * Cross-dialect SQL type translator.
 *
 * Phase 1 — normalize:  source raw type  →  CanonicalType
 * Phase 2 — emit:       CanonicalType    →  target dialect string
 *
 * Supported dialects (case-insensitive):
 *   "oracle", "postgres" | "postgresql",
 *   "mssql" | "sqlserver", "mysql" | "mariadb"
 */

// ─── Canonical type ───────────────────────────────────────────────────────────

export type CanonicalType =
  | { kind: "varchar";     length?: number }
  | { kind: "char";        length?: number }
  | { kind: "text" }
  | { kind: "int";         bits: 2 | 4 | 8 }
  | { kind: "numeric";     precision?: number; scale?: number }
  | { kind: "float";       bits: 4 | 8 }
  | { kind: "boolean" }
  | { kind: "date" }
  | { kind: "time";        precision?: number }
  | { kind: "timestamp";   precision?: number }
  | { kind: "timestamptz"; precision?: number }
  | { kind: "bytes" }
  | { kind: "uuid" }
  | { kind: "json" }
  | { kind: "jsonb" }
  | { kind: "xml" }
  | { kind: "unknown";     original: string }

// ─── Helpers ──────────────────────────────────────────────────────────────────

/** Extract the first parenthesised argument as a number (e.g. VARCHAR2(20) → 20). */
function arg1(u: string): number | undefined {
  const m = u.match(/\(\s*(\d+)/)
  return m ? parseInt(m[1], 10) : undefined
}

/** Extract up to two numeric arguments (e.g. NUMBER(10,2) → [10, 2]). */
function args2(u: string): [number | undefined, number | undefined] {
  const m = u.match(/\(\s*(\d+)\s*(?:,\s*(\d+))?\s*\)/)
  if (!m) return [undefined, undefined]
  return [parseInt(m[1], 10), m[2] !== undefined ? parseInt(m[2], 10) : undefined]
}

function normDialect(d: string): string {
  return d.trim().toLowerCase()
}

// ─── Phase 1: normalize source type to canonical ─────────────────────────────

function normalizeOracle(u: string): CanonicalType {
  if (/^N?VARCHAR2(\s*\(|$)/.test(u))        return { kind: "varchar",   length: arg1(u) }
  if (/^N?CHAR(\s*\(|$)/.test(u))            return { kind: "char",      length: arg1(u) }
  if (/^CLOB$|^NCLOB$|^LONG$/.test(u))       return { kind: "text" }
  if (/^NUMBER(\s*\(|$)/.test(u)) {
    const [p, s] = args2(u)
    if (p === undefined)                      return { kind: "numeric" }
    if (s !== undefined)                      return { kind: "numeric", precision: p, scale: s }
    if (p <= 4)                               return { kind: "int", bits: 2 }
    if (p <= 9)                               return { kind: "int", bits: 4 }
    if (p <= 18)                              return { kind: "int", bits: 8 }
    return { kind: "numeric", precision: p }
  }
  if (/^INTEGER$|^INT$/.test(u))              return { kind: "int", bits: 4 }
  if (/^FLOAT(\s*\(|$)/.test(u))             return { kind: "float", bits: 8 }
  if (/^BINARY_FLOAT$/.test(u))              return { kind: "float", bits: 4 }
  if (/^BINARY_DOUBLE$/.test(u))             return { kind: "float", bits: 8 }
  if (/^DATE$/.test(u))                      return { kind: "timestamp" }   // Oracle DATE has time
  if (/^TIMESTAMP(\s*\(\d+\))? WITH (LOCAL )?TIME ZONE$/.test(u))
                                             return { kind: "timestamptz", precision: arg1(u) }
  if (/^TIMESTAMP(\s*\(|$)/.test(u))        return { kind: "timestamp", precision: arg1(u) }
  if (/^BLOB$|^RAW(\s*\(.*\))?$|^LONG RAW$/.test(u))
                                             return { kind: "bytes" }
  if (/^XMLTYPE$/.test(u))                   return { kind: "xml" }
  if (/^ROWID$/.test(u))                     return { kind: "varchar", length: 18 }
  if (/^INTERVAL/.test(u))                   return { kind: "varchar", length: 40 }
  return { kind: "unknown", original: u }
}

function normalizePostgres(u: string): CanonicalType {
  if (/^(?:CHARACTER VARYING|VARCHAR)(\s*\(|$)/.test(u))
                                             return { kind: "varchar",   length: arg1(u) }
  if (/^(?:CHARACTER|CHAR)(\s*\(|$)/.test(u))
                                             return { kind: "char",      length: arg1(u) }
  if (/^TEXT$/.test(u))                     return { kind: "text" }
  if (/^SMALLINT$|^INT2$/.test(u))          return { kind: "int", bits: 2 }
  if (/^(?:INTEGER|INT|INT4)$/.test(u))     return { kind: "int", bits: 4 }
  if (/^(?:BIGINT|INT8)$/.test(u))          return { kind: "int", bits: 8 }
  if (/^SMALLSERIAL$|^SERIAL2$/.test(u))    return { kind: "int", bits: 2 }
  if (/^SERIAL$|^SERIAL4$/.test(u))         return { kind: "int", bits: 4 }
  if (/^BIGSERIAL$|^SERIAL8$/.test(u))      return { kind: "int", bits: 8 }
  if (/^(?:NUMERIC|DECIMAL)(\s*\(|$)/.test(u)) {
    const [p, s] = args2(u)
    return { kind: "numeric", precision: p, scale: s }
  }
  if (/^(?:REAL|FLOAT4)$/.test(u))          return { kind: "float", bits: 4 }
  if (/^(?:DOUBLE PRECISION|FLOAT8)$/.test(u)) return { kind: "float", bits: 8 }
  if (/^FLOAT(\s*\(|$)/.test(u)) {
    const n = arg1(u)
    return { kind: "float", bits: n !== undefined && n <= 24 ? 4 : 8 }
  }
  if (/^(?:BOOLEAN|BOOL)$/.test(u))         return { kind: "boolean" }
  if (/^DATE$/.test(u))                     return { kind: "date" }
  if (/^TIME(\s*\(\d+\))?( WITHOUT TIME ZONE)?$/.test(u))
                                             return { kind: "time", precision: arg1(u) }
  if (/^TIMETZ$|^TIME(\s*\(\d+\))? WITH TIME ZONE$/.test(u))
                                             return { kind: "time", precision: arg1(u) }
  if (/^TIMESTAMPTZ$|^TIMESTAMP(\s*\(\d+\))? WITH TIME ZONE$/.test(u))
                                             return { kind: "timestamptz", precision: arg1(u) }
  if (/^TIMESTAMP(\s*\(|$)/.test(u))        return { kind: "timestamp", precision: arg1(u) }
  if (/^BYTEA$/.test(u))                    return { kind: "bytes" }
  if (/^UUID$/.test(u))                     return { kind: "uuid" }
  if (/^JSONB$/.test(u))                    return { kind: "jsonb" }
  if (/^JSON$/.test(u))                     return { kind: "json" }
  if (/^XML$/.test(u))                      return { kind: "xml" }
  return { kind: "unknown", original: u }
}

function normalizeMssql(u: string): CanonicalType {
  if (/^N?VARCHAR\s*\(\s*MAX\s*\)/.test(u)) return { kind: "text" }
  if (/^N?VARCHAR(\s*\(|$)/.test(u))        return { kind: "varchar",   length: arg1(u) }
  if (/^N?CHAR(\s*\(|$)/.test(u))           return { kind: "char",      length: arg1(u) }
  if (/^N?TEXT$/.test(u))                   return { kind: "text" }
  if (/^TINYINT$/.test(u))                  return { kind: "int", bits: 2 }
  if (/^SMALLINT$/.test(u))                 return { kind: "int", bits: 2 }
  if (/^(?:INT|INTEGER)$/.test(u))          return { kind: "int", bits: 4 }
  if (/^BIGINT$/.test(u))                   return { kind: "int", bits: 8 }
  if (/^(?:DECIMAL|NUMERIC)(\s*\(|$)/.test(u)) {
    const [p, s] = args2(u)
    return { kind: "numeric", precision: p, scale: s }
  }
  if (/^MONEY$/.test(u))                    return { kind: "numeric", precision: 19, scale: 4 }
  if (/^SMALLMONEY$/.test(u))               return { kind: "numeric", precision: 10, scale: 4 }
  if (/^REAL$/.test(u))                     return { kind: "float", bits: 4 }
  if (/^FLOAT(\s*\(|$)/.test(u)) {
    const n = arg1(u)
    return { kind: "float", bits: n !== undefined && n <= 24 ? 4 : 8 }
  }
  if (/^BIT$/.test(u))                      return { kind: "boolean" }
  if (/^DATE$/.test(u))                     return { kind: "date" }
  if (/^TIME(\s*\(|$)/.test(u))            return { kind: "time", precision: arg1(u) }
  if (/^SMALLDATETIME$/.test(u))            return { kind: "timestamp" }
  if (/^DATETIME2?(\s*\(|$)/.test(u))      return { kind: "timestamp", precision: arg1(u) }
  if (/^DATETIMEOFFSET(\s*\(|$)/.test(u))  return { kind: "timestamptz", precision: arg1(u) }
  if (/^(?:BINARY|VARBINARY)(\s*\(|$)/.test(u)) return { kind: "bytes" }
  if (/^VARBINARY\s*\(\s*MAX\s*\)/.test(u)) return { kind: "bytes" }
  if (/^IMAGE$/.test(u))                    return { kind: "bytes" }
  if (/^UNIQUEIDENTIFIER$/.test(u))         return { kind: "uuid" }
  if (/^XML$/.test(u))                      return { kind: "xml" }
  if (/^SQL_VARIANT$/.test(u))              return { kind: "text" }
  if (/^ROWVERSION$|^TIMESTAMP$/.test(u))  return { kind: "bytes" }
  return { kind: "unknown", original: u }
}

function normalizeMysql(u: string): CanonicalType {
  if (/^VARCHAR(\s*\(|$)/.test(u))          return { kind: "varchar",   length: arg1(u) }
  if (/^CHAR(\s*\(|$)/.test(u))             return { kind: "char",      length: arg1(u) }
  if (/^(?:TINY|MEDIUM|LONG)?TEXT$/.test(u)) return { kind: "text" }
  if (/^TINYINT\s*\(\s*1\s*\)$/.test(u))   return { kind: "boolean" }
  if (/^TINYINT(\s*\(|$)/.test(u))         return { kind: "int", bits: 2 }
  if (/^SMALLINT(\s*\(|$)/.test(u))        return { kind: "int", bits: 2 }
  if (/^MEDIUMINT(\s*\(|$)/.test(u))       return { kind: "int", bits: 4 }
  if (/^(?:INT|INTEGER)(\s*\(|$)/.test(u)) return { kind: "int", bits: 4 }
  if (/^BIGINT(\s*\(|$)/.test(u))          return { kind: "int", bits: 8 }
  if (/^(?:DECIMAL|NUMERIC)(\s*\(|$)/.test(u)) {
    const [p, s] = args2(u)
    return { kind: "numeric", precision: p, scale: s }
  }
  if (/^FLOAT(\s*\(|$)/.test(u))           return { kind: "float", bits: 4 }
  if (/^(?:DOUBLE(?: PRECISION)?)(\s*\(|$)/.test(u)) return { kind: "float", bits: 8 }
  if (/^BIT\s*\(\s*1\s*\)$/.test(u))       return { kind: "boolean" }
  if (/^DATE$/.test(u))                     return { kind: "date" }
  if (/^TIME(\s*\(|$)/.test(u))            return { kind: "time", precision: arg1(u) }
  if (/^DATETIME(\s*\(|$)/.test(u))        return { kind: "timestamp", precision: arg1(u) }
  if (/^TIMESTAMP(\s*\(|$)/.test(u))       return { kind: "timestamptz", precision: arg1(u) }
  if (/^YEAR(\s*\(|$)/.test(u))            return { kind: "int", bits: 2 }
  if (/^(?:TINY|MEDIUM|LONG)?BLOB$/.test(u)) return { kind: "bytes" }
  if (/^(?:VAR)?BINARY(\s*\(|$)/.test(u))  return { kind: "bytes" }
  if (/^ENUM(\s*\(|$)/.test(u))            return { kind: "varchar", length: 255 }
  if (/^SET(\s*\(|$)/.test(u))             return { kind: "varchar", length: 255 }
  if (/^JSON$/.test(u))                     return { kind: "json" }
  return { kind: "unknown", original: u }
}

export function normalizeType(rawType: string, dialect: string): CanonicalType {
  const u = rawType.toUpperCase().trim()
  switch (normDialect(dialect)) {
    case "oracle":                return normalizeOracle(u)
    case "postgres":
    case "postgresql":            return normalizePostgres(u)
    case "mssql":
    case "sqlserver":             return normalizeMssql(u)
    case "mysql":
    case "mariadb":               return normalizeMysql(u)
    default:                      return { kind: "unknown", original: rawType }
  }
}

// ─── Phase 2: emit target dialect string from canonical type ──────────────────

function paren(...ns: (number | undefined)[]): string {
  const filtered = ns.filter((n): n is number => n !== undefined)
  return filtered.length ? `(${filtered.join(",")})` : ""
}

function emitPostgres(c: CanonicalType): string {
  switch (c.kind) {
    case "varchar":     return c.length !== undefined ? `VARCHAR(${c.length})` : "TEXT"
    case "char":        return `CHAR${paren(c.length)}`
    case "text":        return "TEXT"
    case "int":         return c.bits === 2 ? "SMALLINT" : c.bits === 8 ? "BIGINT" : "INTEGER"
    case "numeric":     return `NUMERIC${paren(c.precision, c.scale)}`
    case "float":       return c.bits === 4 ? "REAL" : "DOUBLE PRECISION"
    case "boolean":     return "BOOLEAN"
    case "date":        return "DATE"
    case "time":        return `TIME${paren(c.precision)}`
    case "timestamp":   return `TIMESTAMP${paren(c.precision)}`
    case "timestamptz": return `TIMESTAMPTZ`
    case "bytes":       return "BYTEA"
    case "uuid":        return "UUID"
    case "json":        return "JSONB"
    case "jsonb":       return "JSONB"
    case "xml":         return "XML"
    case "unknown":     return c.original
  }
}

function emitOracle(c: CanonicalType): string {
  switch (c.kind) {
    case "varchar":     return c.length !== undefined ? `VARCHAR2(${c.length})` : "CLOB"
    case "char":        return `CHAR${paren(c.length)}`
    case "text":        return "CLOB"
    case "int":         return c.bits === 2 ? "NUMBER(5)" : c.bits === 8 ? "NUMBER(19)" : "NUMBER(10)"
    case "numeric":     return `NUMBER${paren(c.precision, c.scale)}`
    case "float":       return c.bits === 4 ? "BINARY_FLOAT" : "BINARY_DOUBLE"
    case "boolean":     return "NUMBER(1)"          // Oracle <23c has no BOOLEAN column
    case "date":        return "DATE"
    case "time":        return "VARCHAR2(8)"         // Oracle has no TIME type
    case "timestamp":   return `TIMESTAMP${paren(c.precision)}`
    case "timestamptz": return `TIMESTAMP${paren(c.precision)} WITH TIME ZONE`
    case "bytes":       return "BLOB"
    case "uuid":        return "VARCHAR2(36)"
    case "json":
    case "jsonb":       return "CLOB"
    case "xml":         return "XMLTYPE"
    case "unknown":     return c.original
  }
}

function emitMssql(c: CanonicalType): string {
  switch (c.kind) {
    case "varchar":     return c.length !== undefined ? `VARCHAR(${c.length})` : "NVARCHAR(MAX)"
    case "char":        return `CHAR${paren(c.length)}`
    case "text":        return "NVARCHAR(MAX)"
    case "int":         return c.bits === 2 ? "SMALLINT" : c.bits === 8 ? "BIGINT" : "INT"
    case "numeric":     return `DECIMAL${paren(c.precision, c.scale)}`
    case "float":       return c.bits === 4 ? "REAL" : "FLOAT"
    case "boolean":     return "BIT"
    case "date":        return "DATE"
    case "time":        return `TIME${paren(c.precision)}`
    case "timestamp":   return `DATETIME2${paren(c.precision)}`
    case "timestamptz": return `DATETIMEOFFSET${paren(c.precision)}`
    case "bytes":       return "VARBINARY(MAX)"
    case "uuid":        return "UNIQUEIDENTIFIER"
    case "json":
    case "jsonb":       return "NVARCHAR(MAX)"
    case "xml":         return "XML"
    case "unknown":     return c.original
  }
}

function emitMysql(c: CanonicalType): string {
  switch (c.kind) {
    case "varchar":     return c.length !== undefined ? `VARCHAR(${c.length})` : "LONGTEXT"
    case "char":        return `CHAR${paren(c.length)}`
    case "text":        return "LONGTEXT"
    case "int":         return c.bits === 2 ? "SMALLINT" : c.bits === 8 ? "BIGINT" : "INT"
    case "numeric":     return `DECIMAL${paren(c.precision, c.scale)}`
    case "float":       return c.bits === 4 ? "FLOAT" : "DOUBLE"
    case "boolean":     return "TINYINT(1)"
    case "date":        return "DATE"
    case "time":        return `TIME${paren(c.precision)}`
    case "timestamp":   return `DATETIME${paren(c.precision)}`
    case "timestamptz": return `DATETIME${paren(c.precision)}`  // MySQL has no real TZ type
    case "bytes":       return "LONGBLOB"
    case "uuid":        return "CHAR(36)"
    case "json":
    case "jsonb":       return "JSON"
    case "xml":         return "LONGTEXT"
    case "unknown":     return c.original
  }
}

export function emitType(canonical: CanonicalType, dialect: string): string {
  switch (normDialect(dialect)) {
    case "postgres":
    case "postgresql":  return emitPostgres(canonical)
    case "oracle":      return emitOracle(canonical)
    case "mssql":
    case "sqlserver":   return emitMssql(canonical)
    case "mysql":
    case "mariadb":     return emitMysql(canonical)
    default:
      return canonical.kind === "unknown" ? canonical.original : emitPostgres(canonical)
  }
}

/**
 * Translate a raw SQL type string from one database dialect to another.
 * Returns the original string unchanged when src === tgt or either dialect
 * is unrecognised.
 */
export function translateColumnType(
  rawType: string,
  srcDialect: string,
  tgtDialect: string,
): string {
  if (normDialect(srcDialect) === normDialect(tgtDialect)) return rawType
  const canonical = normalizeType(rawType, srcDialect)
  if (canonical.kind === "unknown") return rawType  // preserve as-is if unrecognised
  return emitType(canonical, tgtDialect)
}
