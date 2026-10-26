export type ConnectorKind = "source" | "sink"
export type ConnectorDatabaseType = "postgres" | "mysql" | "mariadb" | "sqlserver" | "oracle" | "unknown"
type DatabaseType = ConnectorDatabaseType

export type ConnectorConnection = {
  host: string
  port: string
  database: string
  username: string
  jdbcUrl?: string
}

export type ConnectorInsight = {
  kind: ConnectorKind
  databaseType: DatabaseType
  databaseLabel: string
  className: string
  connection: ConnectorConnection
  configuredSchemas: string[]
  configuredTables: string[]
}

export const DATABASE_TYPES: Array<{
  value: DatabaseType
  label: string
  monogram: string
  port: string
  tone: string
}> = [
  { value: "postgres", label: "PostgreSQL", monogram: "PG", port: "5432", tone: "sky" },
  { value: "mysql", label: "MySQL", monogram: "MY", port: "3306", tone: "cyan" },
  { value: "mariadb", label: "MariaDB", monogram: "MA", port: "3306", tone: "amber" },
  { value: "sqlserver", label: "SQL Server", monogram: "MS", port: "1433", tone: "red" },
  { value: "oracle", label: "Oracle", monogram: "OR", port: "1521", tone: "orange" },
]

const SECRET_KEY = /password|secret|token|sasl/i

export const databaseMeta = (type?: string | null) =>
  DATABASE_TYPES.find((item) => item.value === normalizeDatabaseType(type || "")) || {
    value: "unknown" as DatabaseType,
    label: "Database",
    monogram: "DB",
    port: "",
    tone: "slate",
  }

export const normalizeDatabaseType = (value: string): DatabaseType => {
  const v = value.trim().toLowerCase()
  if (v === "postgres" || v === "postgresql") return "postgres"
  if (v === "mysql") return "mysql"
  if (v === "mariadb") return "mariadb"
  if (v === "sqlserver") return "sqlserver"
  if (v === "oracle") return "oracle"
  return "unknown"
}

export const insightFromConfig = (
  config: Record<string, string> = {},
  connectType?: string | null
): ConnectorInsight => {
  const className = String(config["connector.class"] || "").trim()
  const jdbcUrl = String(config["connection.url"] || "").trim()
  const kind: ConnectorKind =
    String(connectType || "").toLowerCase() === "sink" || className.toLowerCase().includes("sink")
      ? "sink"
      : "source"
  let databaseType = inferDatabaseType(className, jdbcUrl)
  const parsed = parseJdbcUrl(jdbcUrl)
  if (databaseType === "unknown" && parsed) databaseType = parsed.databaseType

  const host = firstNonEmpty(
    config["database.hostname"],
    config["database.host"],
    parsed?.host
  )
  const port = firstNonEmpty(config["database.port"], parsed?.port, databaseMeta(databaseType).port)
  const database = firstNonEmpty(
    config["database.dbname"],
    config["database.names"],
    String(config["database.include.list"] || "").split(",")[0],
    parsed?.database
  )
  const username = firstNonEmpty(config["database.user"], config["connection.username"], config["database.username"])

  let configuredSchemas = splitCsv(config["schema.include.list"], parsed?.schema)
  if (!configuredSchemas.length && (databaseType === "mysql" || databaseType === "mariadb")) {
    configuredSchemas = splitCsv(config["database.include.list"])
  }

  return {
    kind,
    databaseType,
    databaseLabel: databaseMeta(databaseType).label,
    className,
    connection: { host, port, database, username, jdbcUrl: jdbcUrl || undefined },
    configuredSchemas,
    configuredTables: splitCsv(config["table.include.list"]),
  }
}

export const maskConfigSecrets = (config: Record<string, string>, reveal: boolean) => {
  if (reveal) return { ...config }
  return Object.fromEntries(
    Object.entries(config).map(([key, value]) => [key, SECRET_KEY.test(key) && value ? "••••••" : value])
  )
}

export const stringifyConfig = (config: Record<string, string>, revealSecrets = true) =>
  JSON.stringify(maskConfigSecrets(config, revealSecrets), null, 2)

const inferDatabaseType = (className: string, jdbcUrl: string): DatabaseType => {
  const c = className.toLowerCase()
  if (c.includes("postgres")) return "postgres"
  if (c.includes("mariadb")) return "mariadb"
  if (c.includes("mysql")) return "mysql"
  if (c.includes("sqlserver")) return "sqlserver"
  if (c.includes("oracle")) return "oracle"
  return parseJdbcUrl(jdbcUrl)?.databaseType || "unknown"
}

const parseJdbcUrl = (raw: string) => {
  const url = raw.trim()
  const hostPath = (rest: string, databaseType: DatabaseType, defaultPort: string) => {
    const [authorityAndPath, query = ""] = rest.split("?", 2)
    const [hostport, path = ""] = authorityAndPath.split("/", 2)
    const [host, port] = splitHostPort(hostport, defaultPort)
    const database = path.split("/")[0] || ""
    const schema = query
      .split("&")
      .map((part) => part.split("="))
      .find(([key]) => key === "currentSchema" || key === "current_schema")?.[1]
    return { databaseType, host, port, database, schema }
  }
  if (url.startsWith("jdbc:postgresql://")) return hostPath(url.slice("jdbc:postgresql://".length), "postgres", "5432")
  if (url.startsWith("jdbc:mysql://")) return hostPath(url.slice("jdbc:mysql://".length), "mysql", "3306")
  if (url.startsWith("jdbc:mariadb://")) return hostPath(url.slice("jdbc:mariadb://".length), "mariadb", "3306")
  if (url.startsWith("jdbc:sqlserver://")) {
    const rest = url.slice("jdbc:sqlserver://".length)
    const [hostport, params = ""] = rest.split(";", 2)
    const [host, port] = splitHostPort(hostport, "1433")
    const database =
      params
        .split(";")
        .map((part) => part.split("="))
        .find(([key]) => key.toLowerCase() === "databasename" || key.toLowerCase() === "database")?.[1] || ""
    return { databaseType: "sqlserver" as DatabaseType, host, port, database, schema: undefined }
  }
  return null
}

const splitHostPort = (hostport: string, defaultPort: string): [string, string] => {
  const idx = hostport.lastIndexOf(":")
  if (idx > 0 && /^\d+$/.test(hostport.slice(idx + 1))) {
    return [hostport.slice(0, idx).replace(/^\[|\]$/g, ""), hostport.slice(idx + 1)]
  }
  return [hostport.replace(/^\[|\]$/g, ""), defaultPort]
}

const firstNonEmpty = (...values: Array<string | undefined>) =>
  values.map((value) => String(value || "").trim()).find(Boolean) || ""

const splitCsv = (...values: Array<string | undefined>) => {
  const out: string[] = []
  for (const value of values) {
    for (const part of String(value || "").split(",")) {
      const trimmed = part.trim()
      if (trimmed && !out.includes(trimmed)) out.push(trimmed)
    }
  }
  return out
}
