// Live CDC readiness (ADR 0019) and database version display. Pure helpers, unit-tested in
// tests/unit/cdcReadiness.test.ts.

/** Product/version as the JDBC driver reports it (`server` on connection tests and readiness). */
export type ServerInfo = { product: string; version: string; major?: number; minor?: number; driver?: string }

/** Stored version of one side, refreshed in the background (ADR 0019 §8). */
export type VersionStamp = ServerInfo & { checked_at?: string | null; changed_at?: string | null; previous?: string | null; error?: string | null }
export type DbVersions = { source?: VersionStamp | null; target?: VersionStamp | null }

/** Tooltip for a version chip: when it was read, an upgrade, or why the last check failed. */
export const versionTitle = (s?: VersionStamp | null, now = Date.now()): string => {
  if (!s) return ""
  const when = (iso?: string | null) => {
    if (!iso) return "never"
    const m = Math.max(0, Math.round((now - Date.parse(iso)) / 60000))
    return m < 1 ? "just now" : m < 60 ? `${m} min ago` : m < 1440 ? `${Math.round(m / 60)} h ago` : `${Math.round(m / 1440)} d ago`
  }
  const lines = [`Checked ${when(s.checked_at)}`]
  if (s.changed_at && s.previous) lines.push(`Changed from ${s.previous} ${when(s.changed_at)}`)
  if (s.error) lines.push(`Last check failed: ${s.error} (showing the last known version)`)
  return lines.join("\n")
}

/** "PostgreSQL 16.15 → PostgreSQL 17.7" for list rows; "" when nothing has been read yet. */
export const versionLine = (v: DbVersions | null | undefined, dbTarget: boolean): string => {
  const src = formatServer(v?.source)
  const tgt = dbTarget ? formatServer(v?.target) : ""
  if (!src && !tgt) return ""
  return dbTarget ? `${src || "?"} → ${tgt || "?"}` : src
}

/** A version changed in the last 7 days (worth a small "updated" mark). */
export const recentlyChanged = (s?: VersionStamp | null, now = Date.now()) =>
  !!s?.changed_at && now - Date.parse(s.changed_at) < 7 * 86400_000

export type ReadinessStatus = "pass" | "warn" | "fail" | "unknown" | "info"
export type LiveCheck = { id: string; title: string; status: ReadinessStatus; detail: string; value?: string; expected?: string; fix?: string }
export type ReadinessReport = {
  engine: string
  capture_mode: string
  applicable: boolean
  server: ServerInfo | null
  platform: string
  checks: LiveCheck[]
  parameters: { name: string; value: string }[]
  fix_script: string
  summary: { pass: number; warn: number; fail: number; unknown: number }
  ready: boolean
  note?: string
  checked_at: string
}

const SQLSERVER_RELEASE: Record<number, string> = { 17: "2025", 16: "2022", 15: "2019", 14: "2017", 13: "2016", 12: "2014", 11: "2012" }

/** "PostgreSQL 17.6", "MariaDB 10.11.6", "SQL Server 2022 (16.0.4135)", "Oracle Database 19c Enterprise Edition". */
export const formatServer = (s?: ServerInfo | null): string => {
  if (!s || (!s.product && !s.version)) return ""
  const product = (s.product || "").trim()
  const raw = (s.version || "").split(/\r?\n/)[0].trim()
  if (/mariadb/i.test(raw)) return `MariaDB ${raw.replace(/-MariaDB.*$/i, "")}`
  if (/microsoft sql server/i.test(product)) {
    const major = s.major || Number(raw.split(".")[0]) || 0
    const short = raw.split(".").slice(0, 3).map((p) => String(Number(p))).join(".")
    return `SQL Server ${SQLSERVER_RELEASE[major] || major}${short ? ` (${short})` : ""}`
  }
  if (/^oracle/i.test(product)) {
    const m = raw.match(/^(Oracle Database \S+(?: [A-Za-z]+ Edition)?)/)
    return m ? m[1] : raw || product
  }
  const version = raw.replace(/\s*\(.*$/, "").replace(/-log$/, "")
  if (!version) return product
  if (product && version.toLowerCase().startsWith(product.toLowerCase())) return version
  return `${product} ${version}`.trim()
}

export const PLATFORM_LABEL: Record<string, string> = {
  amazon_rds: "Amazon RDS",
  amazon_aurora: "Amazon Aurora",
  google_cloud_sql: "Google Cloud SQL",
  azure: "Azure",
  self_managed: "Self-managed",
}

/** Failures first, then warnings and unknowns; passes last. */
export const STATUS_ORDER: Record<ReadinessStatus, number> = { fail: 0, warn: 1, unknown: 2, info: 3, pass: 4 }
export const sortChecks = (checks: LiveCheck[]) => [...checks].sort((a, b) => STATUS_ORDER[a.status] - STATUS_ORDER[b.status])

/** One line for the panel header. */
export const readinessHeadline = (r: ReadinessReport): { tone: "success" | "warning" | "error" | "info"; text: string } => {
  if (!r.applicable) return { tone: "info", text: "Nothing log-based to check for this capture mode." }
  const { fail, warn, unknown } = r.summary
  if (fail) return { tone: "error", text: `Not ready: ${fail} requirement${fail === 1 ? "" : "s"} missing${warn ? `, ${warn} warning${warn === 1 ? "" : "s"}` : ""}.` }
  if (unknown) return { tone: "warning", text: `No blockers found, but ${unknown} check${unknown === 1 ? "" : "s"} could not be read with this login.` }
  if (warn) return { tone: "warning", text: `Ready, with ${warn} recommendation${warn === 1 ? "" : "s"}.` }
  return { tone: "success", text: "Ready for CDC." }
}
