/**
 * Parses one container log line into columns for the live log viewer (pages/logs.vue).
 *
 * Knows the formats of the CDC stack's own services, then falls back to a generic
 * "timestamp, LEVEL, message" reading:
 *   connect  2026-11-12 14:44:51,333 INFO   postgres|orders|streaming  message   [org.apache.Class]
 *   kafka    WARN  2026-11-12 14:41:32,270 [shard 0:main] kafka - create_topics.cc:159 - message
 *   redis    1:M 11 Nov 2026 22:28:47.666 * message
 *   vector   2026-11-12T14:44:54.182128Z  INFO source{component_id=x ...}:file_server: target::path: message
 *
 * The level comes from the level token the line was written with. Words such as "failed"
 * in the message do not make a line an error ("WARN Failed to create topic … already
 * exists" is a warning). Timestamps without a zone are read as UTC, the containers' zone.
 */

export type LogLevel = "error" | "warn" | "info" | "debug"

export interface ParsedLogLine {
  /** Epoch ms, or null when the line has no timestamp (continuations, unknown formats). */
  ts: number | null
  level: LogLevel
  /** The level as written, normalised (ERROR, WARN, INFO, DEBUG, TRACE); "" when inferred. */
  levelLabel: string
  /** Component, connector or logger that wrote the line; "" when unknown. */
  source: string
  message: string
  /** A stack-trace or wrapped line belonging to the line before it. */
  continuation: boolean
  /** Context for the detail view: logger, thread, file, span fields, key=value pairs. */
  fields: Record<string, string>
}

const LEVELS: Record<string, [LogLevel, string]> = {
  FATAL: ["error", "FATAL"],
  CRITICAL: ["error", "FATAL"],
  SEVERE: ["error", "ERROR"],
  ERROR: ["error", "ERROR"],
  ERR: ["error", "ERROR"],
  WARNING: ["warn", "WARN"],
  WARN: ["warn", "WARN"],
  NOTICE: ["info", "INFO"],
  INFO: ["info", "INFO"],
  DEBUG: ["debug", "DEBUG"],
  TRACE: ["debug", "TRACE"],
}
const levelFrom = (token: string | undefined): [LogLevel, string] | null =>
  (token && LEVELS[token.toUpperCase()]) || null

const LEVEL_WORD = "FATAL|CRITICAL|SEVERE|ERROR|ERR|WARNING|WARN|NOTICE|INFO|DEBUG|TRACE"

/** "2026-11-12 14:44:51,333" / "2026-11-12T14:44:54.182128Z" → epoch ms (zone-less = UTC). */
const isoMs = (text: string): number | null => {
  let s = text.trim().replace(" ", "T").replace(",", ".")
  if (!/(Z|[+-]\d\d:?\d\d)$/.test(s)) s += "Z"
  const ms = Date.parse(s)
  return Number.isNaN(ms) ? null : ms
}

const MONTHS: Record<string, number> = {
  Jan: 0, Feb: 1, Mar: 2, Apr: 3, May: 4, Jun: 5, Jul: 6, Aug: 7, Sep: 8, Oct: 9, Nov: 10, Dec: 11,
}

/** key=value and key="quoted value" pairs. */
export const logfmtFields = (text: string): Record<string, string> => {
  const out: Record<string, string> = {}
  for (const m of text.matchAll(/(?:^|\s)([A-Za-z_][\w.-]*)=("(?:[^"\\]|\\.)*"|[^\s,}]+)/g)) {
    out[m[1]] = m[2].startsWith('"') ? m[2].slice(1, -1) : m[2]
  }
  return out
}

/** org.apache.kafka.connect.runtime.WorkerSinkTask → WorkerSinkTask */
const shortClass = (name: string) => name.split(".").pop() || name

const line = (p: Partial<ParsedLogLine> & Pick<ParsedLogLine, "message">): ParsedLogLine => ({
  ts: null,
  level: "info",
  levelLabel: "",
  source: "",
  continuation: false,
  fields: {},
  ...p,
})

// ── Service formats ──────────────────────────────────────────────────────────

const VECTOR = new RegExp(`^(\\d{4}-\\d\\d-\\d\\dT[\\d:.]+Z)\\s+(${LEVEL_WORD})\\s+(.*)$`)
const VECTOR_SPAN = /^(\w+)\{([^}]*)\}((?::\w+)*):\s+(.*)$/
const RUST_TARGET = /^([\w]+(?:::[\w]+)+):\s+(.*)$/

const parseVector = (raw: string): ParsedLogLine | null => {
  const m = VECTOR.exec(raw)
  if (!m) return null
  const [lvl, label] = levelFrom(m[2])!
  let rest = m[3]
  let source = ""
  const fields: Record<string, string> = {}
  const span = VECTOR_SPAN.exec(rest)
  if (span) {
    Object.assign(fields, logfmtFields(span[2]))
    source = fields.component_id || span[1]
    rest = span[4]
  }
  const target = RUST_TARGET.exec(rest)
  if (target) {
    fields.target = target[1]
    source ||= target[1].split("::").pop()!
    rest = target[2]
  }
  return line({ ts: isoMs(m[1]), level: lvl, levelLabel: label, source, message: rest, fields: { ...fields, ...logfmtFields(rest) } })
}

const CONNECT = new RegExp(`^\\[?(\\d{4}-\\d\\d-\\d\\d \\d\\d:\\d\\d:\\d\\d[,.]\\d{3})\\]?\\s+(${LEVEL_WORD})\\s+(.*)$`)
/** Debezium images: "%X{dbz.connectorType}|%X{dbz.connectorName}|%X{dbz.connectorContext}". */
const DBZ_MDC = /^(\S*)\|(\S*)\|(\S*)\s+(.*)$/
/** Kafka Connect "connector.context": "[orders-sink|task-0] ". */
const CONNECT_CONTEXT = /^\[([^\]|]+)\|([^\]]+)\]\s*(.*)$/
const TRAILING_LOGGER = /^(.*?)\s+(?:\[([\w.$]+)\]|\(([\w.$]+):\d+\))\s*$/

const parseConnect = (raw: string): ParsedLogLine | null => {
  const m = CONNECT.exec(raw)
  if (!m) return null
  const [lvl, label] = levelFrom(m[2])!
  let rest = m[3]
  let source = ""
  const fields: Record<string, string> = {}
  const mdc = DBZ_MDC.exec(rest)
  if (mdc) {
    if (mdc[1]) fields.connector_type = mdc[1]
    if (mdc[2]) fields.connector = source = mdc[2]
    if (mdc[3]) fields.context = mdc[3]
    rest = mdc[4]
  }
  const logger = TRAILING_LOGGER.exec(rest)
  if (logger) {
    fields.logger = logger[2] || logger[3]
    rest = logger[1]
  }
  const ctx = CONNECT_CONTEXT.exec(rest)
  if (ctx) {
    fields.connector = source = ctx[1]
    fields.task = ctx[2]
    rest = ctx[3]
  }
  if (!source && fields.logger) source = shortClass(fields.logger)
  return line({ ts: isoMs(m[1]), level: lvl, levelLabel: label, source, message: rest.trim(), fields })
}

const REDPANDA = new RegExp(
  `^(${LEVEL_WORD})\\s+(\\d{4}-\\d\\d-\\d\\d \\d\\d:\\d\\d:\\d\\d,\\d{3})\\s+\\[([^\\]]+)\\]\\s+(\\S+)\\s+-\\s+(?:([\\w./-]+:\\d+)\\s+-\\s+)?(.*)$`,
)

const parseRedpanda = (raw: string): ParsedLogLine | null => {
  const m = REDPANDA.exec(raw)
  if (!m) return null
  const [lvl, label] = levelFrom(m[1])!
  const fields: Record<string, string> = { shard: m[3] }
  if (m[5]) fields.file = m[5]
  return line({ ts: isoMs(m[2]), level: lvl, levelLabel: label, source: m[4], message: m[6], fields })
}

const REDIS = /^(\d+):([A-Z])\s+(\d{1,2}) (\w{3}) (\d{4}) (\d\d):(\d\d):(\d\d)\.(\d{3})\s+([.\-*#])\s+(.*)$/
const REDIS_LEVEL: Record<string, [LogLevel, string]> = {
  ".": ["debug", "DEBUG"],
  "-": ["debug", "DEBUG"],
  "*": ["info", "INFO"],
  "#": ["warn", "WARN"],
}
const REDIS_ROLE: Record<string, string> = { M: "master", S: "replica", C: "child", X: "sentinel" }

/** "1:signal-handler (1794408605) Received SIGTERM scheduling shutdown..." */
const REDIS_SIGNAL = /^(\d+):signal-handler \((\d+)\) (.*)$/

const parseRedis = (raw: string): ParsedLogLine | null => {
  const sig = REDIS_SIGNAL.exec(raw)
  if (sig) {
    return line({ ts: +sig[2] * 1000, level: "warn", levelLabel: "WARN", source: "signal-handler", message: sig[3], fields: { pid: sig[1] } })
  }
  const m = REDIS.exec(raw)
  if (!m || MONTHS[m[4]] === undefined) return null
  const ts = Date.UTC(+m[5], MONTHS[m[4]], +m[3], +m[6], +m[7], +m[8], +m[9])
  const [lvl, label] = REDIS_LEVEL[m[10]]
  return line({
    ts,
    level: lvl,
    levelLabel: label,
    source: REDIS_ROLE[m[2]] || "redis",
    message: m[11],
    fields: { pid: m[1], ...logfmtFields(m[11]) },
  })
}

// ── Generic fallback ─────────────────────────────────────────────────────────

const GENERIC_TS = /^\[?(\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d(?:[.,]\d+)?(?:Z|[+-]\d\d:?\d\d)?)\]?\s*/
const LEVEL_TOKEN = new RegExp(`(?:^|[\\s\\[|])(${LEVEL_WORD})(?=[\\s\\]|:]|$)`)
const LEVEL_KV = /\b(?:level|lvl|severity)=("?)(\w+)\1/i
/** Lines that open an error without a level token: Java exceptions, Rust panics. */
const ERROR_SHAPE = /^(?:Exception in thread|thread '.*' panicked|panic:|[\w.$]+(?:Exception|Error)(?::|$))/
/** Stack frames and wrapped lines. */
const CONTINUATION_SHAPE = /^(?:\s|at |Caused by:|Suppressed:|\.\.\. \d+ more)/

const parseJson = (raw: string): ParsedLogLine | null => {
  if (!raw.startsWith("{")) return null
  let obj: Record<string, unknown>
  try {
    obj = JSON.parse(raw)
  } catch {
    return null
  }
  if (!obj || typeof obj !== "object") return null
  const str = (...keys: string[]) => {
    for (const k of keys) if (obj[k] !== undefined && obj[k] !== null) return String(obj[k])
    return ""
  }
  const lvl = levelFrom(str("level", "lvl", "severity", "log.level"))
  const time = str("timestamp", "time", "ts", "@timestamp")
  const ts = /^\d+(\.\d+)?$/.test(time) ? Number(time) * (Number(time) < 1e12 ? 1000 : 1) : time ? isoMs(time) : null
  const fields: Record<string, string> = {}
  for (const [k, v] of Object.entries(obj)) fields[k] = typeof v === "string" ? v : JSON.stringify(v)
  return line({
    ts,
    level: lvl?.[0] ?? "info",
    levelLabel: lvl?.[1] ?? "",
    source: str("logger", "target", "component", "module", "name"),
    message: str("message", "msg", "event") || raw,
    fields,
  })
}

const parseGeneric = (raw: string, prev?: ParsedLogLine): ParsedLogLine => {
  const json = parseJson(raw)
  if (json) return json
  const tsMatch = GENERIC_TS.exec(raw)
  const ts = tsMatch ? isoMs(tsMatch[1]) : null
  let rest = tsMatch ? raw.slice(tsMatch[0].length) : raw

  // An explicit level near the start of the line, or level=… anywhere.
  const head = rest.slice(0, 48)
  const tok = LEVEL_TOKEN.exec(head)
  let lvl = tok ? levelFrom(tok[1]) : null
  if (tok && lvl) rest = (rest.slice(0, tok.index) + rest.slice(tok.index + tok[0].length)).replace(/^[\s:|\]-]+/, "")
  if (!lvl) {
    const kv = LEVEL_KV.exec(raw)
    lvl = kv ? levelFrom(kv[2]) : null
  }

  // No timestamp, no level, and following a timestamped line: part of the previous event.
  if (!ts && !lvl && prev && (prev.ts !== null || prev.continuation) && (CONTINUATION_SHAPE.test(raw) || ERROR_SHAPE.test(raw) || prev.continuation)) {
    return line({ level: prev.level, levelLabel: "", source: prev.source, message: raw, continuation: true })
  }

  const inferred: LogLevel | null = !lvl && ERROR_SHAPE.test(rest) ? "error" : null
  return line({
    ts,
    level: lvl?.[0] ?? inferred ?? "info",
    levelLabel: lvl?.[1] ?? "",
    message: rest,
    fields: logfmtFields(rest),
  })
}

/**
 * Parses one raw line. Pass the previously parsed line so stack frames and wrapped
 * lines are attached to the event they belong to (same level and source).
 */
export const parseLogLine = (raw: string, prev?: ParsedLogLine): ParsedLogLine => {
  const text = raw.replace(/\r$/, "")
  // Errors the orchestrator injects into the stream itself (docker logs failures).
  if (text.startsWith("[cdc-error] ")) {
    return line({ level: "error", levelLabel: "ERROR", source: "orchestrator", message: text.slice(12) })
  }
  return (
    parseVector(text) ??
    parseConnect(text) ??
    parseRedpanda(text) ??
    parseRedis(text) ??
    parseGeneric(text, prev)
  )
}
