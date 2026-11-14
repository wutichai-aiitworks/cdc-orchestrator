// AI vector sync (ADR 0018): draft ⇄ API shapes and the small rules the editor shows inline.
// Pure functions only (unit-tested in tests/unit/vectorSync.test.ts).

export type VsProvider = "ollama" | "openai" | "hash"
export type ContentMode = "all" | "pick" | "template" | "document"

export type TableDraft = {
  mode: ContentMode
  /** "pick": the columns that make up the text. */
  include_fields: string[]
  /** Never embedded and never in metadata (sensitive columns start here). */
  exclude_fields: string[]
  /** null = every column that is not excluded. */
  metadata_fields: string[] | null
  content_template: string
  /** "document": the column holding file bytes (PDF/DOCX/…), converted by Docling. */
  document_column: string
  /** Optional column with the file name (helps format detection; kept as metadata). */
  filename_column: string
  /** Prefix every document chunk with the row's own text (e.g. title, customer). */
  include_row_text: boolean
  /** Split long text into several vectors. Always on for documents. */
  chunking: { enabled: boolean; max_chars: number; overlap: number }
}

export const DEFAULT_CHUNKING = { enabled: false, max_chars: 1500, overlap: 150 }
/** Columns that usually hold files: offered first as the document column. */
export const FILE_COLUMN_RE = /(file|blob|bytes|content|document|attachment|pdf|data|binary)/i

export type VsDraft = {
  name: string
  description: string
  enabled: boolean
  source_profile_id: string | null
  topics: string[]
  start_from: "earliest" | "latest"
  embedding: { provider: VsProvider; base_url: string; model: string; api_key: string; dimensions: number | null; batch_size: number | null }
  target: { host: string; port: string; database: string; username: string; password: string; sslmode: string; collection: string }
  /** Keyed by table name (last segment of the topic). */
  tables: Record<string, TableDraft>
  delete_mode: "hard" | "soft"
  pk_fields: string[]
}

export const PROVIDERS: { key: VsProvider; label: string; hint: string; defaultUrl: string; defaultModel: string }[] = [
  { key: "ollama", label: "Ollama", hint: "Local models on your own machine or server. Nothing leaves your network.", defaultUrl: "http://host.docker.internal:11434", defaultModel: "bge-m3" },
  { key: "openai", label: "OpenAI-compatible", hint: "OpenAI, Azure-style gateways, vLLM, LM Studio: anything that serves /v1/embeddings.", defaultUrl: "https://api.openai.com/v1", defaultModel: "text-embedding-3-small" },
  { key: "hash", label: "Demo (no model)", hint: "Keyword hashing so you can try the flow without a model. Not semantic: use for demos only.", defaultUrl: "", defaultModel: "hash-256" },
]

/** Embedding models known to work well, shown as quick picks (Ollama names). */
export const SUGGESTED_MODELS: Record<VsProvider, { name: string; note: string }[]> = {
  ollama: [
    { name: "bge-m3", note: "1024-d · multilingual (Thai, English…) · recommended" },
    { name: "nomic-embed-text", note: "768-d · English · fast" },
    { name: "mxbai-embed-large", note: "1024-d · English · strong" },
  ],
  openai: [
    { name: "text-embedding-3-small", note: "1536-d · cheap" },
    { name: "text-embedding-3-large", note: "3072-d · best quality" },
  ],
  hash: [{ name: "hash-256", note: "256-d · not semantic" }],
}

export const SENSITIVE_RE = /(pass(word|wd)?|pwd|secret|token|api[_-]?key|private[_-]?key|salt|hash|otp|pin_?code|ssn|cvv|card[_-]?(no|num|number))/i

/** Columns that look like credentials: excluded by default, with a note saying why. */
export const sensitiveColumns = (cols: string[]) => cols.filter((c) => SENSITIVE_RE.test(c))

export const tableOf = (topic: string) => topic.split(".").pop() || topic

/** A collection name the API accepts, from any sync name. */
export const slug = (name: string) => {
  const s = name.toLowerCase().normalize("NFKD").replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "").slice(0, 63)
  return s || "vectors"
}

export const COLLECTION_RE = /^[A-Za-z0-9_][A-Za-z0-9_.-]{0,62}$/

export const emptyTable = (cols: string[] = []): TableDraft => ({
  mode: "all",
  include_fields: [],
  exclude_fields: sensitiveColumns(cols),
  metadata_fields: null,
  content_template: "",
  document_column: "",
  filename_column: "",
  include_row_text: false,
  chunking: { ...DEFAULT_CHUNKING },
})

export const emptyVsDraft = (): VsDraft => ({
  name: "",
  description: "",
  enabled: true,
  source_profile_id: null,
  topics: [],
  start_from: "earliest",
  embedding: { provider: "ollama", base_url: PROVIDERS[0].defaultUrl, model: PROVIDERS[0].defaultModel, api_key: "", dimensions: null, batch_size: null },
  target: { host: "", port: "5432", database: "", username: "", password: "", sslmode: "", collection: "" },
  tables: {},
  delete_mode: "hard",
  pk_fields: [],
})

/** Demo store started with VECTORSYNC_DEMO=true (docker-compose.sidecars.yaml). */
export const DEMO_TARGET = { host: "cdc-pgvector", port: "5432", database: "vector", username: "vector", password: "vector", sslmode: "" }

// ── templates (Python string.Template rules, same as src/vectorsync.rs) ─────────────────

export const templateError = (tpl: string): string | null => {
  let i = 0
  const ident = (c: string) => /[A-Za-z0-9_]/.test(c)
  while (i < tpl.length) {
    if (tpl[i] !== "$") {
      i++
      continue
    }
    const n = tpl[i + 1]
    if (n === "$") i += 2
    else if (n === "{") {
      const end = tpl.indexOf("}", i + 2)
      const name = end > 0 ? tpl.slice(i + 2, end) : ""
      if (!name || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) return "${…} must hold a column name"
      i = end + 1
    } else if (n && /[A-Za-z_]/.test(n)) {
      i += 2
      while (i < tpl.length && ident(tpl[i])) i++
    } else return "Use $column or ${column}; write $$ for a literal $"
  }
  return null
}

export const templateFields = (tpl: string): string[] => {
  const out = new Set<string>()
  for (const m of tpl.matchAll(/\$(?:\{([A-Za-z_][A-Za-z0-9_]*)\}|([A-Za-z_][A-Za-z0-9_]*))|\$\$/g)) {
    const name = m[1] || m[2]
    if (name) out.add(name)
  }
  return [...out]
}

/** Columns a template names that the table does not have (would dead-letter every row). */
export const unknownTemplateFields = (tpl: string, cols: string[]) => (cols.length ? templateFields(tpl).filter((f) => !cols.includes(f)) : [])

/** A starting template from the first few useful columns: "name: $name · city: $city". */
export const suggestTemplate = (cols: string[], exclude: string[] = []) =>
  cols
    .filter((c) => !exclude.includes(c) && !/^(id|.*_id|created_at|updated_at|_.*)$/i.test(c))
    .slice(0, 6)
    .map((c) => `${c.replace(/_/g, " ")}: \${${c}}`)
    .join("\n")

// ── draft ⇄ API ─────────────────────────────────────────────────────────────────────────

const tableToApi = (t: TableDraft) => {
  const out: Record<string, unknown> = {}
  if (t.mode === "document" && t.document_column) {
    out.document = {
      column: t.document_column,
      ...(t.filename_column ? { filename_column: t.filename_column } : {}),
      ...(t.include_row_text ? { include_row_text: true } : {}),
    }
    out.chunking = { enabled: true, max_chars: t.chunking.max_chars, overlap: t.chunking.overlap }
  } else if (t.chunking.enabled) {
    out.chunking = { enabled: true, max_chars: t.chunking.max_chars, overlap: t.chunking.overlap }
  }
  if (t.exclude_fields.length) out.exclude_fields = [...t.exclude_fields]
  if (t.mode === "pick" && t.include_fields.length) out.include_fields = [...t.include_fields]
  if (t.mode === "template" && t.content_template.trim()) out.content_template = t.content_template
  if (t.metadata_fields) out.metadata_fields = t.metadata_fields.filter((f) => !t.exclude_fields.includes(f))
  return out
}

export const projectionOf = (d: VsDraft) => {
  const tables: Record<string, unknown> = {}
  for (const topic of d.topics) {
    const name = tableOf(topic)
    const t = d.tables[name]
    if (t) tables[name] = tableToApi(t)
  }
  return { default: {}, tables }
}

export const idStrategyOf = (d: VsDraft) => (d.pk_fields.length ? { kind: "table_pk", pk_fields: [...d.pk_fields] } : { kind: "table_pk" })

export const vsToRequest = (d: VsDraft) => ({
  name: d.name.trim(),
  description: d.description.trim(),
  enabled: d.enabled,
  source_profile_id: d.source_profile_id,
  topics: [...d.topics],
  start_from: d.start_from,
  embedding: {
    provider: d.embedding.provider,
    base_url: d.embedding.provider === "hash" ? "" : d.embedding.base_url.trim(),
    model: d.embedding.model.trim(),
    api_key: d.embedding.provider === "openai" ? d.embedding.api_key : "",
    ...(d.embedding.dimensions ? { dimensions: d.embedding.dimensions } : {}),
    ...(d.embedding.batch_size ? { batch_size: d.embedding.batch_size } : {}),
  },
  target: { ...d.target, host: d.target.host.trim(), database: d.target.database.trim(), username: d.target.username.trim(), collection: d.target.collection.trim() },
  projection: projectionOf(d),
  id_strategy: idStrategyOf(d),
  delete_mode: d.delete_mode,
  batch: {},
})

const tableFromApi = (cfg: any): TableDraft => ({
  mode: cfg?.document?.column ? "document" : cfg?.content_template ? "template" : cfg?.include_fields?.length ? "pick" : "all",
  document_column: cfg?.document?.column || "",
  filename_column: cfg?.document?.filename_column || "",
  include_row_text: !!cfg?.document?.include_row_text,
  chunking: {
    enabled: !!cfg?.chunking?.enabled && !cfg?.document?.column,
    max_chars: Number(cfg?.chunking?.max_chars) || DEFAULT_CHUNKING.max_chars,
    overlap: cfg?.chunking?.overlap ?? DEFAULT_CHUNKING.overlap,
  },
  include_fields: [...(cfg?.include_fields || [])],
  exclude_fields: [...(cfg?.exclude_fields || [])],
  metadata_fields: Array.isArray(cfg?.metadata_fields) ? [...cfg.metadata_fields] : null,
  content_template: cfg?.content_template || "",
})

export const fromProfile = (p: any): VsDraft => {
  const d = emptyVsDraft()
  d.name = p.name || ""
  d.description = p.description || ""
  d.enabled = !!p.enabled
  d.source_profile_id = p.source_profile_id || null
  d.topics = [...(p.topics || [])]
  d.start_from = p.start_from === "latest" ? "latest" : "earliest"
  d.embedding = { ...d.embedding, ...p.embedding, dimensions: p.embedding?.dimensions ?? null, batch_size: p.embedding?.batch_size ?? null }
  d.target = { ...d.target, ...p.target }
  for (const [name, cfg] of Object.entries(p.projection?.tables || {})) d.tables[name] = tableFromApi(cfg)
  d.delete_mode = p.delete_mode === "soft" ? "soft" : "hard"
  d.pk_fields = [...(p.id_strategy?.pk_fields || [])]
  return d
}

/** Changes the sidecar must rebuild for (old vectors would not be comparable). */
export const rebuildReasons = (before: VsDraft | null, after: VsDraft): string[] => {
  if (!before) return []
  const out: string[] = []
  const a = before.embedding
  const b = after.embedding
  if (a.provider !== b.provider || a.model.trim() !== b.model.trim() || (a.dimensions || null) !== (b.dimensions || null)) out.push("the embedding model changed")
  if (before.target.collection.trim() !== after.target.collection.trim()) out.push("the collection changed")
  return out
}

// ── validation per wizard step ──────────────────────────────────────────────────────────

export type Step = "source" | "model" | "store" | "documents" | "review"
export const STEPS: { key: Step; label: string }[] = [
  { key: "source", label: "Tables" },
  { key: "model", label: "Embedding model" },
  { key: "store", label: "Vector store" },
  { key: "documents", label: "Documents" },
  { key: "review", label: "Review" },
]

export const stepProblems = (d: VsDraft, step: Step, columns: Record<string, string[]> = {}, docling = false): string[] => {
  const out: string[] = []
  if (step === "source" && !d.topics.length) out.push("Pick at least one table.")
  if (step === "model") {
    if (d.embedding.provider !== "hash" && !d.embedding.base_url.trim()) out.push("Enter the server URL.")
    if (!d.embedding.model.trim()) out.push("Choose a model.")
    if (d.embedding.dimensions !== null && (d.embedding.dimensions < 1 || d.embedding.dimensions > 16000)) out.push("Dimensions must be 1–16000.")
  }
  if (step === "store") {
    if (!d.target.host.trim() || !d.target.database.trim() || !d.target.username.trim()) out.push("Host, database and username are required.")
    if (d.target.port.trim() && !/^\d+$/.test(d.target.port.trim())) out.push("Port must be a number.")
    if (!COLLECTION_RE.test(d.target.collection.trim())) out.push("Collection: letters, digits, _ . - (max 63, starts with a letter, digit or _).")
  }
  if (step === "documents") {
    for (const topic of d.topics) {
      const name = tableOf(topic)
      const t = d.tables[name]
      if (!t) continue
      if (t.mode === "template") {
        if (!t.content_template.trim()) out.push(`${name}: write a template or choose another content mode.`)
        const err = templateError(t.content_template)
        if (err) out.push(`${name}: ${err}`)
        const unknown = unknownTemplateFields(t.content_template, columns[name] || [])
        if (unknown.length) out.push(`${name}: no column named ${unknown.map((u) => `$${u}`).join(", ")}`)
      }
      if (t.mode === "pick" && !t.include_fields.length) out.push(`${name}: pick at least one column for the text.`)
      if (t.mode === "document") {
        if (!docling) out.push(`${name}: Docling is not installed in the sidecar (set VECTORSYNC_DOCLING=true and redeploy).`)
        if (!t.document_column) out.push(`${name}: choose the column that holds the files.`)
      }
      if (t.mode === "document" || t.chunking.enabled) {
        if (t.chunking.max_chars < 200 || t.chunking.max_chars > 20000) out.push(`${name}: chunk size must be 200–20000 characters.`)
        if (t.chunking.overlap < 0 || t.chunking.overlap > t.chunking.max_chars / 2) out.push(`${name}: overlap must be 0 to half the chunk size.`)
      }
    }
  }
  if (step === "review" && !d.name.trim()) out.push("Give the sync a name.")
  return out
}

// ── display ─────────────────────────────────────────────────────────────────────────────

export const STATE_LABEL: Record<string, string> = {
  running: "Running",
  connecting: "Connecting",
  starting: "Starting",
  retrying: "Retrying",
  pending: "Pending",
  paused: "Paused",
  stopped: "Stopped",
  unknown: "Unknown",
  error: "Error",
}

/** Status string AppStatusBadge understands (retrying reads as a warning, not an error). */
export const badgeStatus = (state?: string | null) => (state === "retrying" ? "degraded" : state === "connecting" ? "starting" : state || "unknown")

export const fmtCount = (n: number | null | undefined) => {
  if (n === null || n === undefined) return "—"
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1)}M`
  if (n >= 10_000) return `${(n / 1000).toFixed(n >= 100_000 ? 0 : 1)}k`
  return n.toLocaleString("en-US")
}

export const ago = (iso?: string | null, now = Date.now()) => {
  if (!iso) return "never"
  const s = Math.max(0, Math.round((now - Date.parse(iso)) / 1000))
  if (s < 60) return `${s}s ago`
  if (s < 3600) return `${Math.round(s / 60)}m ago`
  if (s < 86400) return `${Math.round(s / 3600)}h ago`
  return `${Math.round(s / 86400)}d ago`
}
