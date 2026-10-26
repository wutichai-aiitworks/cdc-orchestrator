<script setup lang="ts">
import {
  AlertCircle,
  AlertTriangle,
  ArrowLeft,
  ArrowLeftRight,
  ArrowRight,
  BookMarked,
  CheckCircle2,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Columns2,
  Copy,
  Database,
  Eye,
  FolderPlus,
  GripVertical,
  Loader2,
  Pencil,
  Plus,
  RefreshCw,
  Save,
  Search,
  TableProperties,
  Trash2,
  Wifi,
  X,
} from "@lucide/vue"
import type { DatabaseType, ProfilePayload, SourceTableColumn } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"
import { DB_TYPE_OPTIONS, dbIconPath } from "~/utils/dbIcon"

definePageMeta({ layout: "default" })

const LIVE_CATALOG_TYPES = ["postgres", "postgresql", "mysql", "mariadb", "sqlserver", "mssql", "sqlite", "oracle"]

// ── connection form ───────────────────────────────────────────────
type ConnForm = {
  type: string
  host: string
  port: string
  database: string
  username: string
  password: string
  schema: string
}

const makeEmptyForm = (type = "postgres", port = "5432"): ConnForm => ({
  type,
  host: "",
  port,
  database: "",
  username: "",
  password: "",
  schema: "",
})

const srcForm = reactive<ConnForm>(makeEmptyForm())
const tgtForm = reactive<ConnForm>(makeEmptyForm())

// ── test + catalog state ──────────────────────────────────────────
type ConnState = "idle" | "testing" | "valid" | "invalid"
type CatalogTable = { schema: string; name: string }

const srcConnState = ref<ConnState>("idle")
const srcConnMsg = ref("")
const srcLoading = ref(false)
const srcError = ref("")
const srcSchemas = ref<string[]>([])
const srcTables = ref<CatalogTable[]>([])

const tgtConnState = ref<ConnState>("idle")
const tgtConnMsg = ref("")
const tgtLoading = ref(false)
const tgtError = ref("")
const tgtSchemas = ref<string[]>([])
const tgtTables = ref<CatalogTable[]>([])

const srcReady = computed(() => srcSchemas.value.length > 0 || srcTables.value.length > 0)
const tgtReady = computed(() => tgtSchemas.value.length > 0 || tgtTables.value.length > 0)
const bothReady = computed(() => srcReady.value && tgtReady.value)

// Selected schema for table preview in each panel (null = none selected yet)
const srcViewSchema = ref<string | null>(null)
const tgtViewSchema = ref<string | null>(null)

const srcViewTables = computed(() =>
  srcViewSchema.value
    ? srcTables.value.filter((t) => t.schema === srcViewSchema.value)
    : []
)
const tgtViewTables = computed(() =>
  tgtViewSchema.value
    ? tgtTables.value.filter((t) => t.schema === tgtViewSchema.value)
    : []
)

// Sync comparison table schema filter when a panel schema is picked/cleared
watch(srcViewSchema, (schema) => {
  if (schema) schemaFilter.value = schema
})
watch(tgtViewSchema, (schema) => {
  if (schema) schemaFilter.value = schema
})

// ── Row count cache ───────────────────────────────────────────────────────────
// key = "src|schema\0table" or "tgt|schema\0table"
// value: number = count, null = loading, -1 = error
const rowCountCache  = reactive<Record<string, number | null>>({})
// tracks which "side|schema" schema-batches are in-flight
const schemaCountFetching = reactive<Record<string, boolean>>({})

const countCacheKey = (side: "src" | "tgt", schema: string, name: string) =>
  `${side}|${schema}\0${name}`

const getCount = (side: "src" | "tgt", schema: string, name: string): number | null =>
  rowCountCache[countCacheKey(side, schema, name)] ?? null

// Single batch call — one request returns all table counts for a schema
const fetchSchemaCountsBatch = async (side: "src" | "tgt", schema: string) => {
  const batchKey = `${side}|${schema}`
  if (schemaCountFetching[batchKey]) return    // already in-flight
  schemaCountFetching[batchKey] = true

  // Mark all known tables as loading (null)
  const tables = side === "src"
    ? srcTables.value.filter(t => t.schema === schema)
    : tgtTables.value.filter(t => t.schema === schema)
  tables.forEach(t => {
    const k = countCacheKey(side, t.schema, t.name)
    if (!(k in rowCountCache)) rowCountCache[k] = null
  })

  const conn = side === "src" ? buildSrcConn() : buildTgtConn()
  try {
    const res = await $fetch<{ counts: Record<string, number>; estimated: boolean }>(
      "/api/cdc/compare/schema/counts",
      { method: "POST", body: { connection: conn, schema } },
    )
    // Populate cache from response map
    for (const [name, count] of Object.entries(res.counts ?? {})) {
      rowCountCache[countCacheKey(side, schema, name)] = count
    }
    // Any table not returned by the server → -1 (unknown)
    tables.forEach(t => {
      const k = countCacheKey(side, t.schema, t.name)
      if (rowCountCache[k] === null) rowCountCache[k] = -1
    })
  } catch {
    // Mark all as error
    tables.forEach(t => {
      rowCountCache[countCacheKey(side, t.schema, t.name)] = -1
    })
  } finally {
    schemaCountFetching[batchKey] = false
  }
}

watch(srcViewSchema, (schema) => { if (schema) fetchSchemaCountsBatch("src", schema) })
watch(tgtViewSchema, (schema) => { if (schema) fetchSchemaCountsBatch("tgt", schema) })

const fmtCount = (n: number | null): string => {
  if (n === null) return "…"
  if (n === -1)   return "?"
  return n.toLocaleString()
}

// ── table selection (key = "schema\0name") ────────────────────────
const srcSelectedKeys = ref<Set<string>>(new Set())
const tgtSelectedKeys = ref<Set<string>>(new Set())

const tableKey = (schema: string, name: string) => `${schema}\0${name}`

const srcSelectionCount = computed(() => srcSelectedKeys.value.size)
const tgtSelectionCount = computed(() => tgtSelectedKeys.value.size)

// Toggle one table in the source selection
const toggleSrcTable = (schema: string, name: string) => {
  const k = tableKey(schema, name)
  const s = new Set(srcSelectedKeys.value)
  s.has(k) ? s.delete(k) : s.add(k)
  srcSelectedKeys.value = s
}
const toggleTgtTable = (schema: string, name: string) => {
  const k = tableKey(schema, name)
  const s = new Set(tgtSelectedKeys.value)
  s.has(k) ? s.delete(k) : s.add(k)
  tgtSelectedKeys.value = s
}

// Select / deselect all tables in the currently viewed schema
const selectAllSrcSchema = () => {
  const s = new Set(srcSelectedKeys.value)
  srcViewTables.value.forEach((t) => s.add(tableKey(t.schema, t.name)))
  srcSelectedKeys.value = s
}
const clearAllSrcSchema = () => {
  const s = new Set(srcSelectedKeys.value)
  srcViewTables.value.forEach((t) => s.delete(tableKey(t.schema, t.name)))
  srcSelectedKeys.value = s
}
const selectAllTgtSchema = () => {
  const s = new Set(tgtSelectedKeys.value)
  tgtViewTables.value.forEach((t) => s.add(tableKey(t.schema, t.name)))
  tgtSelectedKeys.value = s
}
const clearAllTgtSchema = () => {
  const s = new Set(tgtSelectedKeys.value)
  tgtViewTables.value.forEach((t) => s.delete(tableKey(t.schema, t.name)))
  tgtSelectedKeys.value = s
}

const srcSchemaSelectedCount = computed(() =>
  srcViewTables.value.filter((t) => srcSelectedKeys.value.has(tableKey(t.schema, t.name))).length
)
const tgtSchemaSelectedCount = computed(() =>
  tgtViewTables.value.filter((t) => tgtSelectedKeys.value.has(tableKey(t.schema, t.name))).length
)

// ── filters ───────────────────────────────────────────────────────
const schemaFilter = ref("__all__")
const tableSearch = ref("")
const statusFilter = ref<"all" | "both" | "source_only" | "target_only">("all")

// ── full diff dialog ──────────────────────────────────────────────
type DiffTarget = { schema: string; name: string } | null
const diffOpen = ref(false)
const diffSrcTable = ref<DiffTarget>(null)
const diffTgtTable = ref<DiffTarget>(null)

// ── inline column diff cache ──────────────────────────────────────
type ColDiffStatus = "match" | "type_diff" | "source_only" | "target_only"
type ColDiffRow = {
  name: string
  sourceCol?: SourceTableColumn
  targetCol?: SourceTableColumn
  status: ColDiffStatus
}
type InlineDiff = { loading: boolean; error: string; rows: ColDiffRow[]; expanded: boolean }
const inlineDiffCache = reactive<Record<string, InlineDiff>>({})

// ── composable ────────────────────────────────────────────────────
const {
  validateSource,
  validateTarget,
  browseSourceCatalog,
  browseTargetCatalog,
  inspectSourceTable,
  inspectTargetTable,
} = useProfilesM3()

// ── helpers ───────────────────────────────────────────────────────
const filled = (v: string) => v.trim().length > 0

const srcLiveCatalogSupported = computed(() => LIVE_CATALOG_TYPES.includes(srcForm.type.toLowerCase()))
const tgtLiveCatalogSupported = computed(() => LIVE_CATALOG_TYPES.includes(tgtForm.type.toLowerCase()))

const canTestSrc = computed(() =>
  srcForm.type === "sqlite"
    ? filled(srcForm.database)
    : filled(srcForm.host) && filled(srcForm.port) && filled(srcForm.database) && filled(srcForm.username)
)
const canTestTgt = computed(() =>
  tgtForm.type === "sqlite"
    ? filled(tgtForm.database)
    : filled(tgtForm.host) && filled(tgtForm.port) && filled(tgtForm.database) && filled(tgtForm.username)
)

const buildSrcPayload = (): ProfilePayload["source"] => ({
  source_type: srcForm.type as DatabaseType,
  connection: {
    host: srcForm.host.trim(),
    port: srcForm.port.trim(),
    database: srcForm.database.trim(),
    username: srcForm.username.trim(),
    password: srcForm.password,
  },
  schema_mode: "all_schemas",
  schemas: srcForm.schema.trim() ? [srcForm.schema.trim()] : [],
  tables: [],
})

const buildTgtPayload = (): NonNullable<ProfilePayload["target"]> => ({
  target_type: tgtForm.type as DatabaseType,
  connection: {
    host: tgtForm.host.trim(),
    port: tgtForm.port.trim(),
    database: tgtForm.database.trim(),
    username: tgtForm.username.trim(),
    password: tgtForm.password,
  },
  schemas: tgtForm.schema.trim() ? [tgtForm.schema.trim()] : [],
})

// ── load catalog (called internally after test or by Reload button) ──
const loadSourceCatalog = async () => {
  srcLoading.value = true
  srcError.value = ""
  srcViewSchema.value = null
  srcSelectedKeys.value = new Set()
  try {
    const res = await browseSourceCatalog(buildSrcPayload())
    srcSchemas.value = (res.schemas || []).map((s) => s.name)
    srcTables.value = (res.tables || []).map((t) => ({ schema: t.schema, name: t.name }))
    if (!srcSchemas.value.length && !srcTables.value.length) {
      srcError.value = res.message || "No schemas or tables found."
    }
  } catch (e: any) {
    srcSchemas.value = []
    srcTables.value = []
    srcError.value = formatApiError(e, "Failed to load source catalog")
  } finally {
    srcLoading.value = false
  }
}

const loadTargetCatalog = async () => {
  tgtLoading.value = true
  tgtError.value = ""
  tgtViewSchema.value = null
  tgtSelectedKeys.value = new Set()
  try {
    const res = await browseTargetCatalog(buildTgtPayload())
    tgtSchemas.value = (res.schemas || []).map((s) => s.name)
    tgtTables.value = (res.tables || []).map((t) => ({ schema: t.schema, name: t.name }))
    if (!tgtSchemas.value.length && !tgtTables.value.length) {
      tgtError.value = res.message || "No schemas or tables found."
    }
  } catch (e: any) {
    tgtSchemas.value = []
    tgtTables.value = []
    tgtError.value = formatApiError(e, "Failed to load target catalog")
  } finally {
    tgtLoading.value = false
  }
}

// ── test connection ───────────────────────────────────────────────
const testSource = async () => {
  srcConnState.value = "testing"
  srcConnMsg.value = ""
  srcError.value = ""
  try {
    const res = await validateSource(buildSrcPayload())
    srcConnState.value = res.valid ? "valid" : "invalid"
    srcConnMsg.value = res.message || ""
    if (res.valid && srcLiveCatalogSupported.value) {
      await loadSourceCatalog()
    }
  } catch (e: any) {
    srcConnState.value = "invalid"
    srcConnMsg.value = formatApiError(e, "Connection test failed")
  }
}

const testTarget = async () => {
  tgtConnState.value = "testing"
  tgtConnMsg.value = ""
  tgtError.value = ""
  try {
    const res = await validateTarget(buildTgtPayload())
    tgtConnState.value = res.valid ? "valid" : "invalid"
    tgtConnMsg.value = res.message || ""
    if (res.valid && tgtLiveCatalogSupported.value) {
      await loadTargetCatalog()
    }
  } catch (e: any) {
    tgtConnState.value = "invalid"
    tgtConnMsg.value = formatApiError(e, "Connection test failed")
  }
}

// ── reset connection state on form change ─────────────────────────
watch(
  () => ({ ...srcForm }),
  () => {
    srcConnState.value = "idle"
    srcConnMsg.value = ""
    srcSchemas.value = []
    srcTables.value = []
    srcError.value = ""
  }
)
watch(
  () => ({ ...tgtForm }),
  () => {
    tgtConnState.value = "idle"
    tgtConnMsg.value = ""
    tgtSchemas.value = []
    tgtTables.value = []
    tgtError.value = ""
  }
)

// ── db type port fill ─────────────────────────────────────────────
watch(
  () => srcForm.type,
  (type) => {
    const opt = DB_TYPE_OPTIONS.find((o) => o.value === type)
    if (opt?.port && (!srcForm.port || DB_TYPE_OPTIONS.some((o) => o.port === srcForm.port))) {
      srcForm.port = opt.port
    }
  }
)
watch(
  () => tgtForm.type,
  (type) => {
    const opt = DB_TYPE_OPTIONS.find((o) => o.value === type)
    if (opt?.port && (!tgtForm.port || DB_TYPE_OPTIONS.some((o) => o.port === tgtForm.port))) {
      tgtForm.port = opt.port
    }
  }
)

// ── compare table logic ───────────────────────────────────────────
type TableRow = {
  schema: string
  name: string
  inSource: boolean
  inTarget: boolean
  status: "both" | "source_only" | "target_only"
}

const allSchemas = computed(() => {
  const set = new Set([...srcSchemas.value, ...tgtSchemas.value])
  return Array.from(set).sort()
})

const tableRows = computed<TableRow[]>(() => {
  const tgtSet = new Set(tgtTables.value.map((t) => `${t.schema}\0${t.name}`))
  const srcSet = new Set(srcTables.value.map((t) => `${t.schema}\0${t.name}`))

  // When the user has made a selection on either side, restrict to those tables.
  const hasSrcSel = srcSelectedKeys.value.size > 0
  const hasTgtSel = tgtSelectedKeys.value.size > 0

  const rows: TableRow[] = []
  for (const t of srcTables.value) {
    const key = `${t.schema}\0${t.name}`
    if (hasSrcSel && !srcSelectedKeys.value.has(key)) continue
    rows.push({ schema: t.schema, name: t.name, inSource: true, inTarget: tgtSet.has(key), status: tgtSet.has(key) ? "both" : "source_only" })
  }
  for (const t of tgtTables.value) {
    const key = `${t.schema}\0${t.name}`
    if (hasTgtSel && !tgtSelectedKeys.value.has(key)) continue
    if (!srcSet.has(key)) {
      rows.push({ schema: t.schema, name: t.name, inSource: false, inTarget: true, status: "target_only" })
    }
  }
  rows.sort((a, b) => `${a.schema}.${a.name}`.localeCompare(`${b.schema}.${b.name}`))
  return rows
})

const filteredRows = computed(() =>
  tableRows.value.filter((row) => {
    if (schemaFilter.value !== "__all__" && row.schema !== schemaFilter.value) return false
    if (statusFilter.value !== "all" && row.status !== statusFilter.value) return false
    if (tableSearch.value.trim()) {
      const q = tableSearch.value.trim().toLowerCase()
      if (!row.name.toLowerCase().includes(q) && !row.schema.toLowerCase().includes(q)) return false
    }
    return true
  })
)

const summary = computed(() => ({
  both: tableRows.value.filter((r) => r.status === "both").length,
  source_only: tableRows.value.filter((r) => r.status === "source_only").length,
  target_only: tableRows.value.filter((r) => r.status === "target_only").length,
}))

// ── inline column diff ────────────────────────────────────────────
const diffKey = (schema: string, name: string) => `${schema}\0${name}`

const computeColDiff = (srcCols: SourceTableColumn[], tgtCols: SourceTableColumn[]): ColDiffRow[] => {
  const srcMap = new Map(srcCols.map((c) => [c.name, c]))
  const tgtMap = new Map(tgtCols.map((c) => [c.name, c]))
  const allNames = [...new Set([...srcMap.keys(), ...tgtMap.keys()])]
  const rows: ColDiffRow[] = allNames.map((name) => {
    const srcCol = srcMap.get(name)
    const tgtCol = tgtMap.get(name)
    let status: ColDiffStatus
    if (srcCol && tgtCol) {
      status = srcCol.data_type === tgtCol.data_type ? "match" : "type_diff"
    } else if (srcCol) {
      status = "source_only"
    } else {
      status = "target_only"
    }
    return { name, sourceCol: srcCol, targetCol: tgtCol, status }
  })
  rows.sort((a, b) => {
    const aPos = a.sourceCol?.position ?? a.targetCol?.position ?? 9999
    const bPos = b.sourceCol?.position ?? b.targetCol?.position ?? 9999
    return aPos - bPos
  })
  return rows
}

const loadInlineDiff = async (row: TableRow) => {
  if (row.status !== "both") return
  const key = diffKey(row.schema, row.name)
  if (!inlineDiffCache[key]) {
    inlineDiffCache[key] = { loading: false, error: "", rows: [], expanded: false }
  }
  const entry = inlineDiffCache[key]
  entry.expanded = !entry.expanded
  if (!entry.expanded || entry.rows.length || entry.loading) return
  entry.loading = true
  entry.error = ""
  try {
    const [srcRes, tgtRes] = await Promise.all([
      inspectSourceTable({ source: buildSrcPayload(), schema: row.schema, name: row.name, limit: 0 }),
      inspectTargetTable({ target: buildTgtPayload(), schema: row.schema, name: row.name, limit: 0 }),
    ])
    entry.rows = computeColDiff(srcRes.columns, tgtRes.columns)
  } catch (e: any) {
    entry.error = formatApiError(e, "Failed to compare columns")
  } finally {
    entry.loading = false
  }
}

const openFullDiff = (row: TableRow) => {
  diffSrcTable.value = { schema: row.schema, name: row.name }
  diffTgtTable.value = { schema: row.schema, name: row.name }
  diffOpen.value = true
}

// ── compare all ───────────────────────────────────────────────────
const comparingAll = ref(false)
const compareAll = async () => {
  comparingAll.value = true
  await Promise.allSettled(
    filteredRows.value
      .filter((r) => r.status === "both")
      .map(async (row) => {
        const key = diffKey(row.schema, row.name)
        if (!inlineDiffCache[key]) {
          inlineDiffCache[key] = { loading: false, error: "", rows: [], expanded: true }
        }
        const entry = inlineDiffCache[key]
        if (entry.rows.length || entry.loading) { entry.expanded = true; return }
        entry.loading = true
        entry.error = ""
        entry.expanded = true
        try {
          const [srcRes, tgtRes] = await Promise.all([
            inspectSourceTable({ source: buildSrcPayload(), schema: row.schema, name: row.name, limit: 0 }),
            inspectTargetTable({ target: buildTgtPayload(), schema: row.schema, name: row.name, limit: 0 }),
          ])
          entry.rows = computeColDiff(srcRes.columns, tgtRes.columns)
        } catch (e: any) {
          entry.error = formatApiError(e, "Failed to compare columns")
        } finally {
          entry.loading = false
        }
      })
  )
  comparingAll.value = false
}

const inlineSummary = (rows: ColDiffRow[]) => ({
  match: rows.filter((r) => r.status === "match").length,
  type_diff: rows.filter((r) => r.status === "type_diff").length,
  source_only: rows.filter((r) => r.status === "source_only").length,
  target_only: rows.filter((r) => r.status === "target_only").length,
})

const hasDiff = (rows: ColDiffRow[]) => {
  const s = inlineSummary(rows)
  return s.type_diff > 0 || s.source_only > 0 || s.target_only > 0
}

// ── compare profiles CRUD ─────────────────────────────────────────
type CompareConn = {
  type: string; host: string; port: string; database: string
  username: string; password: string; schema: string
}
type CompareProfile = {
  id: string; name: string
  source: CompareConn; target: CompareConn
  created_at: string; updated_at: string
}

const savedProfiles = ref<CompareProfile[]>([])
const activeProfileId = ref<string | null>(null)
const profilesLoading = ref(false)
const showSaveDialog = ref(false)
const saveName = ref("")
const saveError = ref("")

const activeProfile = computed(() =>
  savedProfiles.value.find((p) => p.id === activeProfileId.value) ?? null
)

const fetchProfiles = async () => {
  profilesLoading.value = true
  try {
    const res = await $fetch<{ profiles: CompareProfile[] }>("/api/cdc/compare/profiles")
    savedProfiles.value = res.profiles ?? []
  } catch {
    savedProfiles.value = []
  } finally {
    profilesLoading.value = false
  }
}

const loadProfile = (profile: CompareProfile) => {
  activeProfileId.value = profile.id
  Object.assign(srcForm, {
    type: profile.source.type,
    host: profile.source.host,
    port: profile.source.port,
    database: profile.source.database,
    username: profile.source.username,
    password: profile.source.password,
    schema: profile.source.schema,
  })
  Object.assign(tgtForm, {
    type: profile.target.type,
    host: profile.target.host,
    port: profile.target.port,
    database: profile.target.database,
    username: profile.target.username,
    password: profile.target.password,
    schema: profile.target.schema,
  })
}

const openSaveDialog = (mode: "update" | "new" = "new") => {
  if (mode === "update" && activeProfile.value) {
    saveName.value = activeProfile.value.name
  } else {
    // "Save as New" — don't carry the active profile ID
    saveName.value = ""
    activeProfileId.value = null
  }
  saveError.value = ""
  showSaveDialog.value = true
}

const saveProfile = async () => {
  saveError.value = ""
  const name = saveName.value.trim()
  if (!name) { saveError.value = "Name is required"; return }

  const body = {
    name,
    source: {
      type: srcForm.type, host: srcForm.host, port: srcForm.port,
      database: srcForm.database, username: srcForm.username,
      password: srcForm.password, schema: srcForm.schema,
    },
    target: {
      type: tgtForm.type, host: tgtForm.host, port: tgtForm.port,
      database: tgtForm.database, username: tgtForm.username,
      password: tgtForm.password, schema: tgtForm.schema,
    },
  }

  try {
    if (activeProfileId.value) {
      const updated = await $fetch<CompareProfile>(
        `/api/cdc/compare/profiles/${activeProfileId.value}`,
        { method: "PUT", body }
      )
      const idx = savedProfiles.value.findIndex((p) => p.id === updated.id)
      if (idx >= 0) savedProfiles.value[idx] = updated
      else savedProfiles.value.push(updated)
    } else {
      const created = await $fetch<CompareProfile>("/api/cdc/compare/profiles", {
        method: "POST", body,
      })
      savedProfiles.value.push(created)
      activeProfileId.value = created.id
    }
    showSaveDialog.value = false
  } catch (e: any) {
    saveError.value = formatApiError(e, "Failed to save profile")
  }
}

const deleteProfile = async (id: string) => {
  if (!confirm("Delete this compare profile?")) return
  try {
    await $fetch(`/api/cdc/compare/profiles/${id}`, { method: "DELETE" })
    savedProfiles.value = savedProfiles.value.filter((p) => p.id !== id)
    if (activeProfileId.value === id) activeProfileId.value = null
  } catch (e: any) {
    alert(formatApiError(e, "Failed to delete profile"))
  }
}

onMounted(fetchProfiles)

// ── page mode ─────────────────────────────────────────────────────
// 'profiles' = home/list view; 'compare' = configure + compare workspace
const pageMode = ref<"profiles" | "compare">("profiles")

// ── reset ─────────────────────────────────────────────────────────
const resetWorkspace = () => {
  Object.assign(srcForm, makeEmptyForm())
  Object.assign(tgtForm, makeEmptyForm())
  srcConnState.value = "idle"
  srcConnMsg.value = ""
  srcSchemas.value = []
  srcTables.value = []
  srcError.value = ""
  tgtConnState.value = "idle"
  tgtConnMsg.value = ""
  tgtSchemas.value = []
  tgtTables.value = []
  tgtError.value = ""
  schemaFilter.value = "__all__"
  tableSearch.value = ""
  statusFilter.value = "all"
  Object.keys(inlineDiffCache).forEach((k) => delete inlineDiffCache[k])
  diffOpen.value = false
  activeProfileId.value = null
  srcSelectedKeys.value = new Set()
  tgtSelectedKeys.value = new Set()
}

const goHome = () => {
  resetWorkspace()
  pageMode.value = "profiles"
}

const startNew = () => {
  resetWorkspace()
  pageMode.value = "compare"
}

const openProfile = (profile: CompareProfile) => {
  loadProfile(profile)
  pageMode.value = "compare"
}

// ── DDL execution (Create Schema / Create Table / Drop Table on Target) ──────
type DdlDialogMode = "schema" | "table" | "drop" | null

const ddlDialogMode = ref<DdlDialogMode>(null)
const ddlNewSchema = ref("")
const ddlStatement = ref("")        // preview-only (read by user)
const ddlPendingTable = ref<{ schema: string; name: string } | null>(null)
const ddlRunning = ref(false)
const ddlError = ref("")
const ddlSuccess = ref("")

// Build source / target SchemaCompareConn objects from the current form values.
// These match the backend SchemaCompareConn shape: flat { type, host, port, … }.
// Do NOT use buildSrcPayload / buildTgtPayload here — those return the CDC-profile
// shape (source_type + nested connection {}) which the compare endpoints reject.
const buildSrcConn = () => ({
  type: srcForm.type,
  host: srcForm.host,
  port: srcForm.port,
  database: srcForm.database,
  username: srcForm.username,
  password: srcForm.password,
  schema: srcForm.schema,
})

const buildTgtConn = () => ({
  type: tgtForm.type,
  host: tgtForm.host,
  port: tgtForm.port,
  database: tgtForm.database,
  username: tgtForm.username,
  password: tgtForm.password,
  schema: tgtForm.schema,
})

const openCreateSchemaDialog = () => {
  ddlDialogMode.value = "schema"
  ddlNewSchema.value = ""
  ddlStatement.value = ""
  ddlError.value = ""
  ddlSuccess.value = ""
}

const openCreateTableDialog = async (schema: string, tableName: string) => {
  ddlPendingTable.value = { schema, name: tableName }
  ddlDialogMode.value = "table"
  ddlNewSchema.value = schema
  ddlStatement.value = ""
  ddlError.value = ""
  ddlSuccess.value = ""

  // Auto-fetch source columns if not already in cache.
  const key = diffKey(schema, tableName)
  if (!inlineDiffCache[key] || (!inlineDiffCache[key].rows.length && !inlineDiffCache[key].loading)) {
    if (!inlineDiffCache[key]) {
      inlineDiffCache[key] = { loading: true, error: "", rows: [], expanded: false }
    } else {
      inlineDiffCache[key].loading = true
      inlineDiffCache[key].error = ""
    }
    try {
      const srcRes = await inspectSourceTable({
        source: buildSrcPayload(),
        schema,
        name: tableName,
        limit: 0,
      })
      // Map source columns as "source_only" diff rows so the DDL builder can use them.
      inlineDiffCache[key].rows = srcRes.columns.map((col) => ({
        name: col.name,
        status: "source_only" as const,
        sourceCol: col,
      }))
    } catch (e: any) {
      inlineDiffCache[key].error = formatApiError(e, "Failed to load source columns")
    } finally {
      inlineDiffCache[key].loading = false
    }
  }
}


// Build CREATE TABLE DDL from already-loaded inline diff columns.
// Column types are translated from the source dialect to the target dialect.
// Schema auto-creation is handled server-side for all supported target databases.
const buildCreateTableDdl = (schema: string, tableName: string): string => {
  const key = diffKey(schema, tableName)
  const cached = inlineDiffCache[key]
  const cols = (cached?.rows ?? [])
    .filter((c) => c.sourceCol)
    .map((c) => {
      const mappedType = translateColumnType(c.sourceCol!.data_type, srcForm.type, tgtForm.type)
      return `  "${c.sourceCol!.name}" ${mappedType}`
    })
    .join(",\n")
  const targetSchema = ddlNewSchema.value.trim() || schema
  return `CREATE TABLE IF NOT EXISTS "${targetSchema}"."${tableName}" (\n${cols}\n);`
}

const buildCreateSchemaDdl = (): string => {
  const name = ddlNewSchema.value.trim()
  if (!name) return ""
  return `CREATE SCHEMA IF NOT EXISTS "${name}";`
}

// Phases for combined create+copy: 'idle' | 'ddl' | 'copy' | 'done'
const ddlCombinedPhase = ref<"idle" | "ddl" | "copy" | "done">("idle")
const ddlCopyRunning   = ref(false)
const ddlCopyError     = ref("")
const ddlCopyRows      = ref<number | null>(null)

const execDdl = async (statement: string) => {
  ddlRunning.value = true
  ddlError.value = ""
  ddlSuccess.value = ""
  try {
    await $fetch("/api/cdc/compare/target/exec-ddl", {
      method: "POST",
      body: { target: buildTgtConn(), statement },
    })
    ddlSuccess.value = "Executed successfully."
    // Reload target catalog so new schema/table appears.
    await loadTargetCatalog()
  } catch (e) {
    ddlError.value = formatApiError(e, "DDL execution failed")
  } finally {
    ddlRunning.value = false
  }
}

// Combined: create table then immediately copy all rows
const execDdlAndCopy = async () => {
  if (!ddlPendingTable.value) return
  const table    = ddlPendingTable.value
  const tgtSchema = ddlNewSchema.value.trim() || table.schema
  const ddl      = buildCreateTableDdl(table.schema, table.name)

  // ── Phase 1: DDL ──────────────────────────────────────────────
  ddlCombinedPhase.value = "ddl"
  ddlRunning.value  = true
  ddlError.value    = ""
  ddlSuccess.value  = ""
  ddlCopyError.value = ""
  ddlCopyRows.value  = null
  try {
    await $fetch("/api/cdc/compare/target/exec-ddl", {
      method: "POST",
      body: { target: buildTgtConn(), statement: ddl },
    })
    ddlSuccess.value = "Table created."
    await loadTargetCatalog()
  } catch (e: any) {
    ddlError.value = formatApiError(e, "DDL execution failed")
    ddlRunning.value = false
    ddlCombinedPhase.value = "idle"
    return
  }
  ddlRunning.value = false

  // ── Phase 2: Copy data ─────────────────────────────────────────
  ddlCombinedPhase.value = "copy"
  ddlCopyRunning.value   = true
  try {
    // Use source schema for reading, target schema for writing (same key in spec).
    // If schemas differ, the API will write to target using the tgtSchema.
    const res = await $fetch<{ tables: CopyTableResult[]; total_rows_copied: number }>(
      "/api/cdc/compare/data/copy",
      {
        method: "POST",
        body: {
          source: buildSrcConn(),
          target: { ...buildTgtConn(), schema: tgtSchema },
          tables: [{ schema: table.schema, name: table.name }],
          batch_size: 500,
          truncate_before_copy: false,
          force_direct_copy: false,
        },
      },
    )
    const result = res.tables?.[0]
    if (result?.status === "error") {
      ddlCopyError.value = result.error ?? "Copy failed"
    } else {
      ddlCopyRows.value = res.total_rows_copied ?? result?.rows_copied ?? 0
    }
  } catch (e: any) {
    ddlCopyError.value = formatApiError(e, "Data copy failed")
  } finally {
    ddlCopyRunning.value   = false
    ddlCombinedPhase.value = "done"
  }
}

const resetDdlCombined = () => {
  ddlCombinedPhase.value = "idle"
  ddlCopyRunning.value   = false
  ddlCopyError.value     = ""
  ddlCopyRows.value      = null
}

const closeDdlDialog = () => {
  ddlDialogMode.value = null
  ddlPendingTable.value = null
  ddlNewSchema.value = ""
  ddlStatement.value = ""
  ddlError.value = ""
  ddlSuccess.value = ""
  resetDdlCombined()
}

const openDropTableDialog = (schema: string, name: string) => {
  ddlPendingTable.value = { schema, name }
  ddlDialogMode.value = "drop"
  ddlNewSchema.value = schema
  ddlError.value = ""
  ddlSuccess.value = ""
}

const buildDropTableDdl = (schema: string, name: string): string => {
  return `DROP TABLE "${schema}"."${name}";`
}

// ── Data Migration ─────────────────────────────────────────────────────────────

type CopyTableResult = {
  schema: string
  name: string
  rows_copied: number
  status: "ok" | "error"
  error?: string
}

const copyDialogOpen = ref(false)
const copyTable = ref<{ schema: string; name: string } | null>(null)
const copyTruncate = ref(false)
const copyBatchSize = ref(500)
const copyForce = ref(false)
const copyRunning = ref(false)
const copyResults = ref<CopyTableResult[]>([])
const copyError = ref("")

const openCopyDialog = (schema: string, name: string) => {
  copyTable.value = { schema, name }
  copyTruncate.value = false
  copyBatchSize.value = 500
  copyForce.value = false
  copyResults.value = []
  copyError.value = ""
  copyDialogOpen.value = true
}

const closeCopyDialog = () => {
  copyDialogOpen.value = false
  copyTable.value = null
  copyForce.value = false
  copyResults.value = []
  copyError.value = ""
}

const runCopyData = async () => {
  if (!copyTable.value) return
  copyRunning.value = true
  copyResults.value = []
  copyError.value = ""
  try {
    const res = await $fetch<{ tables: CopyTableResult[]; total_rows_copied: number; errors: number }>(
      "/api/cdc/compare/data/copy",
      {
        method: "POST",
        body: {
          source: buildSrcConn(),
          target: buildTgtConn(),
          tables: [{ schema: copyTable.value.schema, name: copyTable.value.name }],
          batch_size: copyBatchSize.value,
          truncate_before_copy: copyTruncate.value,
          force_direct_copy: copyForce.value,
        },
      },
    )
    copyResults.value = res.tables ?? []
    if (res.errors > 0) {
      copyError.value = `${res.errors} table(s) failed — see details below.`
    }
  } catch (e: any) {
    copyError.value = formatApiError(e, "Data copy failed")
  } finally {
    copyRunning.value = false
  }
}

// ── Panel drag-to-create (source table list → target panel) ────────────────────
const panelDragTable = ref<{ schema: string; name: string } | null>(null)
const panelDropActive = ref(false)

const startPanelDrag = (e: DragEvent, schema: string, name: string) => {
  panelDragTable.value = { schema, name }
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = "copy"
    e.dataTransfer.setData("text/plain", `${schema}.${name}`)
  }
}

const endPanelDrag = () => {
  panelDragTable.value = null
  panelDropActive.value = false
}

const onPanelDragOver = (e: DragEvent) => {
  if (!panelDragTable.value) return
  e.preventDefault()
  if (e.dataTransfer) e.dataTransfer.dropEffect = "copy"
  panelDropActive.value = true
}

const onPanelDragLeave = (e: DragEvent) => {
  // Only clear when leaving the card itself, not its children
  if ((e.currentTarget as Element)?.contains(e.relatedTarget as Node)) return
  panelDropActive.value = false
}

const onPanelDrop = (e: DragEvent) => {
  e.preventDefault()
  panelDropActive.value = false
  if (panelDragTable.value) {
    openCreateTableDialog(panelDragTable.value.schema, panelDragTable.value.name)
  }
  panelDragTable.value = null
}

// ── Select-to-create (checkbox multi-select → batch create on target) ──────────
const selectedCreateKeys = ref<Set<string>>(new Set())

const isCreateSelected = (row: TableRow) =>
  selectedCreateKeys.value.has(diffKey(row.schema, row.name))

const toggleCreateSelect = (row: TableRow) => {
  const key = diffKey(row.schema, row.name)
  const next = new Set(selectedCreateKeys.value)
  next.has(key) ? next.delete(key) : next.add(key)
  selectedCreateKeys.value = next
}

const selectAllSourceOnly = () => {
  selectedCreateKeys.value = new Set(
    filteredRows.value
      .filter((r) => r.status === "source_only")
      .map((r) => diffKey(r.schema, r.name))
  )
}

const clearCreateSelection = () => {
  selectedCreateKeys.value = new Set()
}

// Batch create state
type BatchCreateItem = {
  schema: string
  name: string
  status: "pending" | "running" | "done" | "error"
  message: string
}
const batchCreateOpen    = ref(false)
const batchCreateItems   = ref<BatchCreateItem[]>([])
const batchCreateRunning = ref(false)
const batchCreateDone    = ref(false)

const openBatchCreateDialog = async () => {
  const rows = filteredRows.value.filter(
    (r) => r.status === "source_only" && selectedCreateKeys.value.has(diffKey(r.schema, r.name))
  )
  batchCreateItems.value = rows.map((r) => ({
    schema: r.schema,
    name: r.name,
    status: "pending",
    message: "",
  }))
  batchCreateRunning.value = false
  batchCreateDone.value    = false
  batchCreateOpen.value    = true

  // Pre-load source columns for all selected rows in parallel
  await Promise.allSettled(rows.map((r) => loadSourceColumnsForDdl(r.schema, r.name)))
}

// Extracted helper — loads source columns into inlineDiffCache for DDL building
const loadSourceColumnsForDdl = async (schema: string, name: string) => {
  const key = diffKey(schema, name)
  if (inlineDiffCache[key]?.rows.length) return
  if (!inlineDiffCache[key]) {
    inlineDiffCache[key] = { loading: true, error: "", rows: [], expanded: false }
  } else {
    inlineDiffCache[key].loading = true
    inlineDiffCache[key].error   = ""
  }
  try {
    const srcRes = await inspectSourceTable({
      source: buildSrcPayload(),
      schema,
      name,
      limit: 0,
    })
    inlineDiffCache[key].rows = srcRes.columns.map((col) => ({
      name:      col.name,
      status:    "source_only" as const,
      sourceCol: col,
    }))
  } catch (e: any) {
    inlineDiffCache[key].error = formatApiError(e, "Failed to load source columns")
  } finally {
    inlineDiffCache[key].loading = false
  }
}

const runBatchCreate = async () => {
  batchCreateRunning.value = true
  for (const item of batchCreateItems.value) {
    item.status  = "running"
    item.message = ""
    const ddl = buildCreateTableDdl(item.schema, item.name)
    try {
      await $fetch("/api/cdc/compare/target/exec-ddl", {
        method: "POST",
        body: { target: buildTgtConn(), statement: ddl },
      })
      item.status  = "done"
      item.message = "Created"
    } catch (e: any) {
      item.status  = "error"
      item.message = formatApiError(e, "Failed")
    }
  }
  batchCreateRunning.value = false
  batchCreateDone.value    = true
  clearCreateSelection()
  await loadTargetCatalog()
}

const closeBatchCreate = () => {
  batchCreateOpen.value = false
  batchCreateItems.value = []
}

// ── Table Data Viewer ──────────────────────────────────────────────────────────

type ViewTableRow = { schema: string; name: string; status: string }

const viewDialogOpen   = ref(false)
const viewTable        = ref<ViewTableRow | null>(null)
const viewSide         = ref<"source" | "target">("source")
const viewPage         = ref(0)
const VIEW_PAGE_SIZE   = 50
const viewSearch       = ref("")
const viewSearchInput  = ref("")   // debounced from viewSearch
const viewLoading      = ref(false)
const viewError        = ref("")
const viewColumns      = ref<string[]>([])
const viewRows         = ref<any[][]>([])
const viewHasMore      = ref(false)

let viewSearchTimer: ReturnType<typeof setTimeout> | null = null

const openViewDialog = (row: ViewTableRow) => {
  viewTable.value = row
  viewSide.value  = row.status === "target_only" ? "target" : "source"
  viewPage.value  = 0
  viewSearch.value      = ""
  viewSearchInput.value = ""
  viewColumns.value = []
  viewRows.value    = []
  viewHasMore.value = false
  viewError.value   = ""
  viewDialogOpen.value = true
  loadViewPage()
}

const closeViewDialog = () => {
  viewDialogOpen.value = false
  viewTable.value = null
}

const loadViewPage = async () => {
  if (!viewTable.value) return
  viewLoading.value = true
  viewError.value   = ""
  try {
    const conn = viewSide.value === "source" ? buildSrcConn() : buildTgtConn()
    const res = await $fetch<{
      columns: string[]
      rows: any[][]
      page: number
      page_size: number
      has_more: boolean
    }>("/api/cdc/compare/table/preview", {
      method: "POST",
      body: {
        connection: conn,
        schema:     viewTable.value.schema,
        table:      viewTable.value.name,
        page:       viewPage.value,
        page_size:  VIEW_PAGE_SIZE,
        search:     viewSearch.value.trim() || null,
      },
    })
    viewColumns.value = res.columns
    viewRows.value    = res.rows
    viewHasMore.value = res.has_more
  } catch (e: any) {
    viewError.value = formatApiError(e, "Failed to load table data")
  } finally {
    viewLoading.value = false
  }
}

const viewPrevPage = () => {
  if (viewPage.value > 0) { viewPage.value--; loadViewPage() }
}
const viewNextPage = () => {
  if (viewHasMore.value) { viewPage.value++; loadViewPage() }
}

const onViewSideChange = (side: "source" | "target") => {
  viewSide.value  = side
  viewPage.value  = 0
  viewSearch.value      = ""
  viewSearchInput.value = ""
  loadViewPage()
}

const onViewSearchInput = (val: string) => {
  viewSearchInput.value = val
  if (viewSearchTimer) clearTimeout(viewSearchTimer)
  viewSearchTimer = setTimeout(() => {
    viewSearch.value = val
    viewPage.value   = 0
    loadViewPage()
  }, 400)
}

const viewRowStart = computed(() => viewPage.value * VIEW_PAGE_SIZE + 1)
const viewRowEnd   = computed(() => viewPage.value * VIEW_PAGE_SIZE + viewRows.value.length)

// Decide which sides (tabs) to show in the viewer based on table status.
const viewAvailableSides = computed<Array<"source" | "target">>(() => {
  if (!viewTable.value) return ["source"]
  if (viewTable.value.status === "source_only") return ["source"]
  if (viewTable.value.status === "target_only") return ["target"]
  return ["source", "target"]
})

// Format a cell value for display (null → italic dash, objects → JSON).
const formatCell = (val: any): string => {
  if (val === null || val === undefined) return ""
  if (typeof val === "object") return JSON.stringify(val)
  return String(val)
}
const isNullCell = (val: any): boolean => val === null || val === undefined

</script>

<template>
  <div class="page-container max-w-7xl">

    <!-- ════════════════════════════════════════════════════════════
         MODE: PROFILES HOME
         ════════════════════════════════════════════════════════════ -->
    <template v-if="pageMode === 'profiles'">
      <!-- Header -->
      <div class="mb-6 flex items-start justify-between gap-4">
        <div>
          <h1 class="text-xl font-bold">Schema Comparisons</h1>
          <p class="mt-1 text-sm text-surface-400">Manage saved comparison profiles. Open one to configure connections and compare table structures.</p>
        </div>
        <AppButton variant="primary" @click="startNew">
          <Plus class="h-4 w-4" :stroke-width="2" />
          New Comparison
        </AppButton>
      </div>

      <!-- Loading -->
      <div v-if="profilesLoading" class="flex items-center justify-center py-20">
        <Loader2 class="h-6 w-6 animate-spin text-brand-400" :stroke-width="1.75" />
        <span class="ml-2 text-surface-400">Loading profiles…</span>
      </div>

      <!-- Empty state -->
      <div v-else-if="savedProfiles.length === 0" class="flex flex-col items-center gap-4 rounded-2xl border border-dashed border-surface-700/60 py-20 text-center">
        <div class="flex h-14 w-14 items-center justify-center rounded-2xl bg-brand-500/10">
          <ArrowLeftRight class="h-7 w-7 text-brand-400" :stroke-width="1.5" />
        </div>
        <div>
          <p class="text-base font-semibold">No comparisons yet</p>
          <p class="mt-1 text-sm text-surface-400">Create your first comparison to start comparing schema structures between databases.</p>
        </div>
        <AppButton variant="primary" @click="startNew">
          <Plus class="h-4 w-4" :stroke-width="2" />
          New Comparison
        </AppButton>
      </div>

      <!-- Profile cards grid -->
      <div v-else class="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
        <!-- Existing profile cards -->
        <div
          v-for="p in savedProfiles"
          :key="p.id"
          class="group relative flex flex-col gap-3 rounded-xl border border-surface-700/40 bg-surface-900 p-4 transition-all hover:border-brand-500/40 hover:shadow-2xl"
        >
          <!-- Card header -->
          <div class="flex items-start justify-between gap-2">
            <div class="min-w-0">
              <p class="truncate font-semibold text-surface-100">{{ p.name }}</p>
              <p class="mt-0.5 text-xs text-surface-500">Saved comparison profile</p>
            </div>
            <!-- Delete -->
            <button
              type="button"
              class="shrink-0 rounded-lg p-1.5 text-surface-600 opacity-0 transition-all group-hover:opacity-100 hover:bg-red-500/10 hover:text-red-400"
              title="Delete profile"
              @click="deleteProfile(p.id)"
            >
              <Trash2 class="h-4 w-4" :stroke-width="1.75" />
            </button>
          </div>

          <!-- Connection summary -->
          <div class="flex flex-col gap-1">
            <!-- Source -->
            <div class="flex items-center gap-2.5 rounded-lg border-l-2 border-emerald-500/60 bg-emerald-500/5 px-3 py-2">
              <img
                :src="dbIconPath(p.source.type)"
                :alt="p.source.type"
                class="h-6 w-6 shrink-0 rounded object-contain"
              />
              <div class="min-w-0 flex-1">
                <p class="text-[10px] font-semibold uppercase tracking-wide text-emerald-500/80">Source</p>
                <p class="truncate text-xs font-medium text-surface-200">
                  <span class="text-surface-300">{{ p.source.host }}</span>
                  <span class="text-surface-500">/{{ p.source.database }}</span>
                </p>
              </div>
            </div>
            <!-- Flow arrow -->
            <div class="flex items-center gap-1 px-2 py-0.5">
              <span class="h-px flex-1 bg-surface-700/60" />
              <ArrowRight class="h-3 w-3 shrink-0 text-surface-500" :stroke-width="1.5" />
              <span class="h-px flex-1 bg-surface-700/60" />
            </div>
            <!-- Target -->
            <div class="flex items-center gap-2.5 rounded-lg border-l-2 border-teal-500/60 bg-teal-500/5 px-3 py-2">
              <img
                :src="dbIconPath(p.target.type)"
                :alt="p.target.type"
                class="h-6 w-6 shrink-0 rounded object-contain"
              />
              <div class="min-w-0 flex-1">
                <p class="text-[10px] font-semibold uppercase tracking-wide text-teal-400/80">Target</p>
                <p class="truncate text-xs font-medium text-surface-200">
                  <span class="text-surface-300">{{ p.target.host }}</span>
                  <span class="text-surface-500">/{{ p.target.database }}</span>
                </p>
              </div>
            </div>
          </div>

          <!-- Open button -->
          <AppButton variant="primary" class="w-full mt-1" @click="openProfile(p)">
            <ArrowLeftRight class="h-4 w-4" :stroke-width="1.75" />
            Open &amp; Compare
          </AppButton>
        </div>

        <!-- "New Comparison" add card -->
        <button
          type="button"
          class="flex flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-surface-700/50 bg-transparent p-8 text-surface-500 transition-all hover:border-brand-500/40 hover:bg-brand-500/5 hover:text-brand-400"
          @click="startNew"
        >
          <div class="flex h-12 w-12 items-center justify-center rounded-xl bg-surface-800/60">
            <Plus class="h-6 w-6" :stroke-width="1.75" />
          </div>
          <span class="text-sm font-medium">New Comparison</span>
        </button>
      </div>
    </template>

    <!-- ════════════════════════════════════════════════════════════
         MODE: COMPARE WORKSPACE
         ════════════════════════════════════════════════════════════ -->
    <template v-else>
      <!-- Breadcrumb + workspace header -->
      <div class="mb-5 flex flex-wrap items-center justify-between gap-3">
        <div class="flex items-center gap-3">
          <button
            type="button"
            class="flex items-center gap-1.5 text-sm text-surface-400 transition-colors hover:text-surface-200"
            @click="goHome"
          >
            <ArrowLeft class="h-4 w-4" :stroke-width="1.75" />
            Comparisons
          </button>
          <span class="text-surface-600">/</span>
          <span class="flex items-center gap-2 text-sm font-semibold text-surface-200">
            <template v-if="activeProfile">
              <img :src="dbIconPath(activeProfile.source.type)" :alt="activeProfile.source.type" class="h-4 w-4 rounded object-contain" />
              <ArrowRight class="h-3 w-3 text-surface-500" :stroke-width="1.5" />
              <img :src="dbIconPath(activeProfile.target.type)" :alt="activeProfile.target.type" class="h-4 w-4 rounded object-contain" />
            </template>
            {{ activeProfile ? activeProfile.name : 'New Comparison' }}
          </span>
        </div>
        <div class="flex items-center gap-2">
          <AppButton v-if="activeProfileId" variant="secondary" size="sm" @click="openSaveDialog('update')">
            <Save class="h-3.5 w-3.5" :stroke-width="1.75" />
            Update Profile
          </AppButton>
          <AppButton variant="secondary" size="sm" @click="openSaveDialog('new')">
            <Save class="h-3.5 w-3.5" :stroke-width="1.75" />
            Save as New
          </AppButton>
          <AppButton size="sm" @click="resetWorkspace">
            <X class="h-3.5 w-3.5" :stroke-width="1.75" />
            Reset
          </AppButton>
        </div>
      </div>

      <!-- ── Save dialog ── -->
      <Teleport to="body">
        <div v-if="showSaveDialog" class="modal-overlay z-50" @click.self="showSaveDialog = false">
          <AppCard class="w-full max-w-sm animate-slide-up space-y-4 !p-5">
            <div class="flex items-center justify-between">
              <div>
                <p class="text-sm font-semibold">{{ activeProfileId ? 'Update Profile' : 'Save as New Profile' }}</p>
                <p v-if="activeProfileId" class="text-xs text-surface-400 mt-0.5">Overwrite the current profile settings.</p>
                <p v-else class="text-xs text-surface-400 mt-0.5">Save the current connection settings as a reusable profile.</p>
              </div>
              <button type="button" class="text-surface-400 hover:text-foreground" @click="showSaveDialog = false">
                <X class="h-4 w-4" :stroke-width="1.75" />
              </button>
            </div>
            <label>
              <span class="auth-label">Profile name</span>
              <AppInput
                v-model="saveName"
                placeholder="e.g. Prod Oracle → Stage PostgreSQL"
                autofocus
                @keyup.enter="saveProfile"
              />
            </label>
            <p v-if="saveError" class="auth-error">{{ saveError }}</p>
            <div class="flex justify-end gap-2">
              <AppButton variant="secondary" @click="showSaveDialog = false">Cancel</AppButton>
              <AppButton variant="primary" @click="saveProfile">
                <Save class="h-4 w-4" :stroke-width="1.75" />
                {{ activeProfileId ? 'Update' : 'Save' }}
              </AppButton>
            </div>
          </AppCard>
        </div>
      </Teleport>

      <!-- ── Connection panels ── -->
      <div class="mb-6 grid grid-cols-1 gap-5 lg:grid-cols-2">
      <!-- ── SOURCE panel ── -->
      <AppCard class="flex flex-col gap-5">
        <!-- Panel header -->
        <div class="flex items-center gap-2.5">
          <div class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-brand-500/15">
            <Database class="h-4 w-4 text-brand-500" :stroke-width="1.75" />
          </div>
          <div>
            <p class="text-sm font-semibold">Source</p>
            <p class="text-xs text-muted-foreground">Database to compare from</p>
          </div>
          <!-- Connection status badge -->
          <div class="ml-auto flex items-center gap-1.5">
            <AppStatusBadge v-if="srcConnState === 'testing'" status="testing" label="Testing…" />
            <AppStatusBadge v-else-if="srcConnState === 'valid'" status="connected" label="Connected" />
            <AppStatusBadge v-else-if="srcConnState === 'invalid'" status="failed" label="Failed" />
          </div>
        </div>

        <!-- DB type picker -->
        <div>
          <span class="auth-label">Database type</span>
          <div class="mt-2 flex flex-wrap gap-1.5">
            <button
              v-for="opt in DB_TYPE_OPTIONS"
              :key="opt.value"
              type="button"
              class="flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition-all"
              :class="srcForm.type === opt.value
                ? 'border-brand-500 bg-brand-500/10 text-foreground shadow-sm'
                : 'border-border bg-card text-muted-foreground hover:border-brand-500/40 hover:bg-brand-500/5'"
              @click="srcForm.type = opt.value; srcForm.port = opt.port"
            >
              <img :src="opt.icon" :alt="opt.label" class="h-4 w-4 object-contain" />
              {{ opt.label }}
            </button>
          </div>
        </div>

        <!-- Connection fields -->
        <div v-if="srcForm.type !== 'sqlite'" class="grid grid-cols-2 gap-3">
          <label class="col-span-2">
            <span class="auth-label">Host</span>
            <AppInput v-model="srcForm.host" placeholder="localhost" autocomplete="off" />
          </label>
          <label>
            <span class="auth-label">Port</span>
            <AppInput v-model="srcForm.port" inputmode="numeric" placeholder="5432" />
          </label>
          <label>
            <span class="auth-label">Database</span>
            <AppInput v-model="srcForm.database" placeholder="mydb" autocomplete="off" />
          </label>
          <label>
            <span class="auth-label">Username</span>
            <AppInput v-model="srcForm.username" placeholder="postgres" autocomplete="off" />
          </label>
          <label>
            <span class="auth-label">Password</span>
            <AppInput v-model="srcForm.password" type="password" placeholder="••••••••" autocomplete="new-password" />
          </label>
          <label class="col-span-2">
            <span class="auth-label">Schema <span class="font-normal text-surface-400">(optional — blank = all schemas)</span></span>
            <AppInput v-model="srcForm.schema" placeholder="e.g. public, bookings, HR" autocomplete="off" />
          </label>
        </div>
        <div v-else>
          <label>
            <span class="auth-label">Database file path</span>
            <AppInput v-model="srcForm.database" placeholder="/data/app.db" />
          </label>
        </div>

        <!-- Live catalog warning -->
        <p
          v-if="!srcLiveCatalogSupported"
          class="rounded-xl border border-yellow-500/30 bg-yellow-500/10 p-3 text-xs text-yellow-800 dark:text-yellow-300"
        >
          Live catalog browsing is not available for
          <span class="font-semibold capitalize">{{ srcForm.type }}</span>.
          Supported: PostgreSQL, MySQL, MariaDB, SQL Server, SQLite, and Oracle
          (Oracle requires DataService configured with <code class="font-mono">ojdbc8.jar</code>).
        </p>

        <!-- Action buttons -->
        <div class="flex flex-wrap gap-2 border-t border-border pt-4">
          <AppButton
            variant="primary"
            :disabled="srcConnState === 'testing' || !canTestSrc || !srcLiveCatalogSupported"
            @click="testSource"
          >
            <Wifi class="h-4 w-4" :stroke-width="1.75" />
            {{ srcConnState === 'testing' ? 'Testing…' : 'Test Connection' }}
          </AppButton>
          <AppButton
            :disabled="srcLoading || srcConnState !== 'valid' || !srcLiveCatalogSupported"
            @click="loadSourceCatalog"
          >
            <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': srcLoading }" :stroke-width="1.75" />
            {{ srcLoading ? 'Loading…' : srcReady ? 'Reload Schemas' : 'Load Schemas' }}
          </AppButton>
        </div>

        <!-- Test result message -->
        <p v-if="srcConnMsg && srcConnState !== 'idle'" class="text-sm" :class="srcConnState === 'valid' ? 'text-emerald-600 dark:text-emerald-400' : 'text-red-700 dark:text-red-400'">
          {{ srcConnMsg }}
        </p>
        <p v-if="srcError" class="text-sm text-red-700 dark:text-red-400">{{ srcError }}</p>

        <!-- Schema / table preview -->
        <div v-if="srcReady" class="rounded-xl border border-emerald-500/20 bg-emerald-500/5 p-3 space-y-2">
          <p class="text-xs font-semibold text-emerald-700 dark:text-emerald-400">
            {{ srcSchemas.length }} {{ srcSchemas.length === 1 ? 'schema' : 'schemas' }} ·
            {{ srcTables.length }} {{ srcTables.length === 1 ? 'table' : 'tables' }} found
            <span v-if="srcViewSchema" class="ml-1 font-normal text-emerald-600 dark:text-emerald-500">— click a schema to browse its tables</span>
          </p>
          <!-- Schema chips — clickable -->
          <div class="flex flex-wrap gap-1.5">
            <button
              v-for="schema in srcSchemas"
              :key="schema"
              type="button"
              class="rounded-lg border px-2 py-0.5 text-[11px] font-medium transition-all"
              :class="srcViewSchema === schema
                ? 'border-emerald-500 bg-emerald-500/20 text-emerald-700 dark:text-emerald-300'
                : 'border-surface-700/60 bg-surface-800/40 text-surface-300 hover:border-emerald-500/50 hover:bg-emerald-500/10'"
              @click="srcViewSchema = srcViewSchema === schema ? null : schema"
            >
              {{ schema }}
              <span class="ml-1 opacity-60">{{ srcTables.filter(t => t.schema === schema).length }}</span>
              <span
                v-if="srcTables.filter(t => t.schema === schema && srcSelectedKeys.has(tableKey(t.schema, t.name))).length > 0"
                class="ml-1 font-semibold text-brand-400"
              >✓{{ srcTables.filter(t => t.schema === schema && srcSelectedKeys.has(tableKey(t.schema, t.name))).length }}</span>
            </button>
          </div>
          <!-- Table selection list for selected schema -->
          <div v-if="srcViewSchema" class="mt-1 space-y-1.5">
            <!-- Header row -->
            <div class="flex items-center justify-between">
              <p class="text-[11px] font-medium text-surface-300">
                <span class="text-surface-200">{{ srcViewSchema }}</span>
                — {{ srcViewTables.length }} tables
                <span v-if="srcSchemaSelectedCount > 0" class="ml-1 font-semibold text-brand-400">
                  ({{ srcSchemaSelectedCount }} selected)
                </span>
              </p>
              <div class="flex gap-1.5">
                <button type="button" class="text-[10px] text-brand-400 hover:underline" @click="selectAllSrcSchema">All</button>
                <span class="text-[10px] text-surface-600">·</span>
                <button type="button" class="text-[10px] text-surface-400 hover:underline" @click="clearAllSrcSchema">None</button>
              </div>
            </div>
            <!-- Scrollable checkable list -->
            <div class="max-h-52 overflow-y-auto rounded-lg border border-surface-700/40 bg-surface-950/60">
              <div
                v-for="table in srcViewTables"
                :key="table.name"
                draggable="true"
                class="group flex cursor-grab items-center gap-2 border-b border-surface-700/20 px-2 py-1.5 last:border-0 hover:bg-surface-800/40 active:cursor-grabbing"
                :class="[
                  srcSelectedKeys.has(tableKey(table.schema, table.name)) ? 'bg-brand-500/5' : '',
                  panelDragTable?.schema === table.schema && panelDragTable?.name === table.name ? 'opacity-40' : '',
                ]"
                :title="'Drag to target panel to create, or click checkbox to select'"
                @dragstart="startPanelDrag($event, table.schema, table.name)"
                @dragend="endPanelDrag"
              >
                <!-- Drag handle — always visible, hints draggability -->
                <GripVertical class="h-3.5 w-3.5 shrink-0 text-surface-600 group-hover:text-surface-400 transition-colors" :stroke-width="1.75" />
                <input
                  type="checkbox"
                  class="h-3.5 w-3.5 shrink-0 cursor-pointer accent-brand-500"
                  :checked="srcSelectedKeys.has(tableKey(table.schema, table.name))"
                  @change="toggleSrcTable(table.schema, table.name)"
                  @click.stop
                />
                <Database class="h-3 w-3 shrink-0 text-surface-500" :stroke-width="1.75" />
                <span class="flex-1 truncate text-[12px]" :class="srcSelectedKeys.has(tableKey(table.schema, table.name)) ? 'text-surface-100 font-medium' : 'text-surface-300'">
                  {{ table.name }}
                </span>
                <!-- Row count badge -->
                <span
                  class="ml-auto shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] tabular-nums"
                  :class="getCount('src', table.schema, table.name) === null
                    ? 'text-surface-600 animate-pulse'
                    : getCount('src', table.schema, table.name) === -1
                      ? 'text-surface-600'
                      : 'bg-surface-800/60 text-surface-400'"
                >
                  {{ fmtCount(getCount('src', table.schema, table.name)) }}
                </span>
              </div>
              <div v-if="srcViewTables.length === 0" class="px-3 py-2 text-[12px] text-surface-400">
                No tables found in this schema.
              </div>
            </div>
          </div>
          <p v-else class="text-[11px] text-surface-400">Click a schema above to select tables for comparison.</p>
          <!-- Total selection badge -->
          <p v-if="srcSelectionCount > 0" class="text-[11px] font-medium text-brand-400">
            {{ srcSelectionCount }} table{{ srcSelectionCount === 1 ? '' : 's' }} selected for comparison
          </p>
        </div>
      </AppCard>

      <!-- ── TARGET panel ── -->
      <AppCard
        class="relative flex flex-col gap-5 transition-all duration-150"
        :class="panelDropActive ? 'ring-2 ring-brand-500 ring-inset' : panelDragTable ? 'ring-1 ring-brand-500/40 ring-inset' : ''"
        @dragover="onPanelDragOver"
        @dragleave="onPanelDragLeave"
        @drop="onPanelDrop"
      >
        <!-- Drop overlay — shown while a source table is being dragged -->
        <Transition
          enter-active-class="transition-opacity duration-150"
          enter-from-class="opacity-0"
          enter-to-class="opacity-100"
          leave-active-class="transition-opacity duration-100"
          leave-from-class="opacity-100"
          leave-to-class="opacity-0"
        >
          <div
            v-if="panelDragTable"
            class="pointer-events-none absolute inset-0 z-10 flex flex-col items-center justify-center gap-3 rounded-xl"
            :class="panelDropActive
              ? 'bg-brand-500/12'
              : 'bg-surface-900/0'"
          >
            <div
              class="flex flex-col items-center gap-2 rounded-2xl border-2 border-dashed px-8 py-6 text-center transition-colors duration-150"
              :class="panelDropActive
                ? 'border-brand-500 bg-brand-500/15 text-brand-300'
                : 'border-brand-500/50 bg-surface-900/80 text-brand-400/70'"
            >
              <div class="flex items-center gap-2">
                <img :src="dbIconPath(srcForm.type)" :alt="srcForm.type" class="h-7 w-7 rounded object-contain" />
                <ArrowRight class="h-5 w-5" :stroke-width="2" />
                <img :src="dbIconPath(tgtForm.type)" :alt="tgtForm.type" class="h-7 w-7 rounded object-contain" />
              </div>
              <div class="text-sm font-semibold">
                <template v-if="panelDropActive">Release to create on target</template>
                <template v-else>Drop here to create</template>
              </div>
              <div class="max-w-[180px] truncate font-mono text-xs opacity-80">
                {{ panelDragTable.schema }}.{{ panelDragTable.name }}
              </div>
            </div>
          </div>
        </Transition>
        <!-- Panel header -->
        <div class="flex items-center gap-2.5">
          <div class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-surface-700/50">
            <Database class="h-4 w-4 text-surface-300" :stroke-width="1.75" />
          </div>
          <div>
            <p class="text-sm font-semibold">Target</p>
            <p class="text-xs text-muted-foreground">Database to compare against</p>
          </div>
          <!-- Connection status badge -->
          <div class="ml-auto flex items-center gap-1.5">
            <AppStatusBadge v-if="tgtConnState === 'testing'" status="testing" label="Testing…" />
            <AppStatusBadge v-else-if="tgtConnState === 'valid'" status="connected" label="Connected" />
            <AppStatusBadge v-else-if="tgtConnState === 'invalid'" status="failed" label="Failed" />
          </div>
        </div>

        <!-- DB type picker -->
        <div>
          <span class="auth-label">Database type</span>
          <div class="mt-2 flex flex-wrap gap-1.5">
            <button
              v-for="opt in DB_TYPE_OPTIONS"
              :key="opt.value"
              type="button"
              class="flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition-all"
              :class="tgtForm.type === opt.value
                ? 'border-brand-500 bg-brand-500/10 text-foreground shadow-sm'
                : 'border-border bg-card text-muted-foreground hover:border-brand-500/40 hover:bg-brand-500/5'"
              @click="tgtForm.type = opt.value; tgtForm.port = opt.port"
            >
              <img :src="opt.icon" :alt="opt.label" class="h-4 w-4 object-contain" />
              {{ opt.label }}
            </button>
          </div>
        </div>

        <!-- Connection fields -->
        <div v-if="tgtForm.type !== 'sqlite'" class="grid grid-cols-2 gap-3">
          <label class="col-span-2">
            <span class="auth-label">Host</span>
            <AppInput v-model="tgtForm.host" placeholder="localhost" autocomplete="off" />
          </label>
          <label>
            <span class="auth-label">Port</span>
            <AppInput v-model="tgtForm.port" inputmode="numeric" placeholder="5432" />
          </label>
          <label>
            <span class="auth-label">Database</span>
            <AppInput v-model="tgtForm.database" placeholder="mydb" autocomplete="off" />
          </label>
          <label>
            <span class="auth-label">Username</span>
            <AppInput v-model="tgtForm.username" placeholder="postgres" autocomplete="off" />
          </label>
          <label>
            <span class="auth-label">Password</span>
            <AppInput v-model="tgtForm.password" type="password" placeholder="••••••••" autocomplete="new-password" />
          </label>
          <label class="col-span-2">
            <span class="auth-label">Schema <span class="font-normal text-surface-400">(optional — blank = all schemas)</span></span>
            <AppInput v-model="tgtForm.schema" placeholder="e.g. public, bookings, HR" autocomplete="off" />
          </label>
        </div>
        <div v-else>
          <label>
            <span class="auth-label">Database file path</span>
            <AppInput v-model="tgtForm.database" placeholder="/data/app.db" />
          </label>
        </div>

        <!-- Live catalog warning -->
        <p
          v-if="!tgtLiveCatalogSupported"
          class="rounded-xl border border-yellow-500/30 bg-yellow-500/10 p-3 text-xs text-yellow-800 dark:text-yellow-300"
        >
          Live catalog browsing is not available for
          <span class="font-semibold capitalize">{{ tgtForm.type }}</span>.
          Supported: PostgreSQL, MySQL, MariaDB, SQL Server, SQLite, and Oracle
          (Oracle requires DataService configured with <code class="font-mono">ojdbc8.jar</code>).
        </p>

        <!-- Action buttons -->
        <div class="flex flex-wrap gap-2 border-t border-border pt-4">
          <AppButton
            variant="primary"
            :disabled="tgtConnState === 'testing' || !canTestTgt || !tgtLiveCatalogSupported"
            @click="testTarget"
          >
            <Wifi class="h-4 w-4" :stroke-width="1.75" />
            {{ tgtConnState === 'testing' ? 'Testing…' : 'Test Connection' }}
          </AppButton>
          <AppButton
            :disabled="tgtLoading || tgtConnState !== 'valid' || !tgtLiveCatalogSupported"
            @click="loadTargetCatalog"
          >
            <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': tgtLoading }" :stroke-width="1.75" />
            {{ tgtLoading ? 'Loading…' : tgtReady ? 'Reload Schemas' : 'Load Schemas' }}
          </AppButton>
          <AppButton
            variant="secondary"
            :disabled="tgtConnState !== 'valid'"
            @click="openCreateSchemaDialog"
          >
            <FolderPlus class="h-4 w-4" :stroke-width="1.75" />
            Create Schema
          </AppButton>
        </div>

        <!-- Test result message -->
        <p v-if="tgtConnMsg && tgtConnState !== 'idle'" class="text-sm" :class="tgtConnState === 'valid' ? 'text-emerald-600 dark:text-emerald-400' : 'text-red-700 dark:text-red-400'">
          {{ tgtConnMsg }}
        </p>
        <p v-if="tgtError" class="text-sm text-red-700 dark:text-red-400">{{ tgtError }}</p>

        <!-- Schema / table preview -->
        <div v-if="tgtReady" class="rounded-xl border border-emerald-500/20 bg-emerald-500/5 p-3 space-y-2">
          <p class="text-xs font-semibold text-emerald-700 dark:text-emerald-400">
            {{ tgtSchemas.length }} {{ tgtSchemas.length === 1 ? 'schema' : 'schemas' }} ·
            {{ tgtTables.length }} {{ tgtTables.length === 1 ? 'table' : 'tables' }} found
          </p>
          <!-- Schema chips — clickable -->
          <div class="flex flex-wrap gap-1.5">
            <button
              v-for="schema in tgtSchemas"
              :key="schema"
              type="button"
              class="rounded-lg border px-2 py-0.5 text-[11px] font-medium transition-all"
              :class="tgtViewSchema === schema
                ? 'border-emerald-500 bg-emerald-500/20 text-emerald-700 dark:text-emerald-300'
                : 'border-surface-700/60 bg-surface-800/40 text-surface-300 hover:border-emerald-500/50 hover:bg-emerald-500/10'"
              @click="tgtViewSchema = tgtViewSchema === schema ? null : schema"
            >
              {{ schema }}
              <span class="ml-1 opacity-60">{{ tgtTables.filter(t => t.schema === schema).length }}</span>
              <span
                v-if="tgtTables.filter(t => t.schema === schema && tgtSelectedKeys.has(tableKey(t.schema, t.name))).length > 0"
                class="ml-1 font-semibold text-brand-400"
              >✓{{ tgtTables.filter(t => t.schema === schema && tgtSelectedKeys.has(tableKey(t.schema, t.name))).length }}</span>
            </button>
          </div>
          <!-- Table selection list for selected schema -->
          <div v-if="tgtViewSchema" class="mt-1 space-y-1.5">
            <!-- Header row -->
            <div class="flex items-center justify-between">
              <p class="text-[11px] font-medium text-surface-300">
                <span class="text-surface-200">{{ tgtViewSchema }}</span>
                — {{ tgtViewTables.length }} tables
                <span v-if="tgtSchemaSelectedCount > 0" class="ml-1 font-semibold text-brand-400">
                  ({{ tgtSchemaSelectedCount }} selected)
                </span>
              </p>
              <div class="flex gap-1.5">
                <button type="button" class="text-[10px] text-brand-400 hover:underline" @click="selectAllTgtSchema">All</button>
                <span class="text-[10px] text-surface-600">·</span>
                <button type="button" class="text-[10px] text-surface-400 hover:underline" @click="clearAllTgtSchema">None</button>
              </div>
            </div>
            <!-- Scrollable checkable list -->
            <div class="max-h-52 overflow-y-auto rounded-lg border border-surface-700/40 bg-surface-950/60">
              <label
                v-for="table in tgtViewTables"
                :key="table.name"
                class="flex cursor-pointer items-center gap-2.5 border-b border-surface-700/20 px-3 py-1.5 last:border-0 hover:bg-surface-800/40"
                :class="tgtSelectedKeys.has(tableKey(table.schema, table.name)) ? 'bg-brand-500/5' : ''"
              >
                <input
                  type="checkbox"
                  class="h-3.5 w-3.5 accent-brand-500"
                  :checked="tgtSelectedKeys.has(tableKey(table.schema, table.name))"
                  @change="toggleTgtTable(table.schema, table.name)"
                />
                <Database class="h-3 w-3 shrink-0 text-surface-500" :stroke-width="1.75" />
                <span class="flex-1 truncate text-[12px]" :class="tgtSelectedKeys.has(tableKey(table.schema, table.name)) ? 'text-surface-100 font-medium' : 'text-surface-300'">
                  {{ table.name }}
                </span>
                <!-- Row count badge -->
                <span
                  class="ml-auto shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] tabular-nums"
                  :class="getCount('tgt', table.schema, table.name) === null
                    ? 'text-surface-600 animate-pulse'
                    : getCount('tgt', table.schema, table.name) === -1
                      ? 'text-surface-600'
                      : 'bg-surface-800/60 text-surface-400'"
                >
                  {{ fmtCount(getCount('tgt', table.schema, table.name)) }}
                </span>
              </label>
              <div v-if="tgtViewTables.length === 0" class="px-3 py-2 text-[12px] text-surface-400">
                No tables found in this schema.
              </div>
            </div>
          </div>
          <p v-else class="text-[11px] text-surface-400">Click a schema above to select tables for comparison.</p>
          <!-- Total selection badge -->
          <p v-if="tgtSelectionCount > 0" class="text-[11px] font-medium text-brand-400">
            {{ tgtSelectionCount }} table{{ tgtSelectionCount === 1 ? '' : 's' }} selected for comparison
          </p>
        </div>
      </AppCard>
    </div>

    <!-- ── Prompt to load both sides ── -->
    <AppStateBlock
      v-if="!srcReady && !tgtReady"
      kind="empty"
      message="Test and load both connections above to start comparing tables."
    />
    <div
      v-else-if="srcReady && !tgtReady"
      class="mb-6 rounded-xl border border-yellow-500/30 bg-yellow-500/10 p-4 text-sm text-yellow-800 dark:text-yellow-300"
    >
      Source loaded — test and load the <strong>Target</strong> connection to start comparing.
    </div>
    <div
      v-else-if="!srcReady && tgtReady"
      class="mb-6 rounded-xl border border-yellow-500/30 bg-yellow-500/10 p-4 text-sm text-yellow-800 dark:text-yellow-300"
    >
      Target loaded — test and load the <strong>Source</strong> connection to start comparing.
    </div>

    <!-- ── Compare section (both ready) ── -->
    <template v-if="bothReady">
      <!-- Selection context banner -->
      <div v-if="srcSelectionCount > 0 || tgtSelectionCount > 0" class="mb-3 flex flex-wrap items-center gap-2 rounded-xl border border-brand-500/20 bg-brand-500/5 px-3 py-2">
        <span class="text-xs font-medium text-brand-400">Comparing selected tables:</span>
        <span v-if="srcSelectionCount > 0" class="rounded-md bg-brand-500/10 px-2 py-0.5 text-xs text-brand-400">
          Source: {{ srcSelectionCount }} table{{ srcSelectionCount === 1 ? '' : 's' }}
        </span>
        <span v-if="tgtSelectionCount > 0" class="rounded-md bg-brand-500/10 px-2 py-0.5 text-xs text-brand-400">
          Target: {{ tgtSelectionCount }} table{{ tgtSelectionCount === 1 ? '' : 's' }}
        </span>
        <button
          class="ml-auto text-[11px] text-surface-400 hover:text-surface-200"
          @click="srcSelectedKeys = new Set(); tgtSelectedKeys = new Set()"
        >
          Clear selection
        </button>
      </div>

      <!-- Summary filter chips -->
      <div v-if="tableRows.length" class="mb-4 flex flex-wrap items-center gap-2">
        <button
          class="rounded-lg border px-3 py-1 text-xs font-medium transition-colors"
          :class="statusFilter === 'all' ? 'border-brand-500 bg-brand-500/10 text-brand-400' : 'border-surface-700 text-surface-400 hover:border-brand-500/40 hover:text-surface-200'"
          @click="statusFilter = 'all'"
        >
          All · {{ tableRows.length }}
        </button>
        <button
          class="rounded-lg border px-3 py-1 text-xs font-medium transition-colors"
          :class="statusFilter === 'both' ? 'border-emerald-500 bg-emerald-500/10 text-emerald-400' : 'border-surface-700 text-surface-400 hover:border-emerald-500/40 hover:text-surface-200'"
          @click="statusFilter = statusFilter === 'both' ? 'all' : 'both'"
        >
          In both · {{ summary.both }}
        </button>
        <button
          v-if="summary.source_only"
          class="rounded-lg border px-3 py-1 text-xs font-medium transition-colors"
          :class="statusFilter === 'source_only' ? 'border-red-500 bg-red-500/10 text-red-400' : 'border-surface-700 text-surface-400 hover:border-red-500/40 hover:text-surface-200'"
          @click="statusFilter = statusFilter === 'source_only' ? 'all' : 'source_only'"
        >
          Source only · {{ summary.source_only }}
        </button>
        <button
          v-if="summary.target_only"
          class="rounded-lg border px-3 py-1 text-xs font-medium transition-colors"
          :class="statusFilter === 'target_only' ? 'border-blue-500 bg-blue-500/10 text-blue-400' : 'border-surface-700 text-surface-400 hover:border-blue-500/40 hover:text-surface-200'"
          @click="statusFilter = statusFilter === 'target_only' ? 'all' : 'target_only'"
        >
          Target only · {{ summary.target_only }}
        </button>
      </div>

      <!-- Filter bar -->
      <div class="mb-4 flex flex-wrap items-end gap-3">
        <label class="min-w-[180px]">
          <span class="auth-label">Schema</span>
          <AppSelect v-model="schemaFilter">
            <option value="__all__">All schemas</option>
            <option v-for="schema in allSchemas" :key="schema" :value="schema">{{ schema }}</option>
          </AppSelect>
        </label>
        <label class="min-w-[200px] flex-1">
          <span class="auth-label">Search tables</span>
          <div class="relative">
            <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" :stroke-width="1.75" />
            <AppInput v-model="tableSearch" placeholder="Filter by table or schema…" class="pl-9" />
          </div>
        </label>
        <div class="flex items-end">
          <AppButton :disabled="comparingAll || summary.both === 0" @click="compareAll">
            <ArrowLeftRight class="h-4 w-4" :class="{ 'animate-spin': comparingAll }" :stroke-width="1.75" />
            {{ comparingAll ? 'Comparing…' : 'Compare all' }}
          </AppButton>
        </div>
      </div>

      <!-- Select-to-create action bar — shown when source-only rows are selected -->
      <Transition
        enter-active-class="transition-all duration-200 ease-out"
        enter-from-class="opacity-0 translate-y-2"
        enter-to-class="opacity-100 translate-y-0"
        leave-active-class="transition-all duration-150 ease-in"
        leave-from-class="opacity-100 translate-y-0"
        leave-to-class="opacity-0 translate-y-2"
      >
        <div
          v-if="selectedCreateKeys.size > 0"
          class="mb-3 flex items-center gap-3 rounded-xl border border-brand-500/30 bg-brand-500/8 px-4 py-3"
        >
          <!-- DB flow -->
          <div class="flex shrink-0 items-center gap-1.5">
            <img :src="dbIconPath(srcForm.type)" :alt="srcForm.type" class="h-5 w-5 rounded object-contain" />
            <ArrowRight class="h-3.5 w-3.5 shrink-0 text-brand-400" :stroke-width="2" />
            <img :src="dbIconPath(tgtForm.type)" :alt="tgtForm.type" class="h-5 w-5 rounded object-contain" />
          </div>
          <!-- Count + hint -->
          <div class="flex-1 text-sm text-surface-200">
            <span class="font-semibold text-brand-300">{{ selectedCreateKeys.size }}</span>
            table{{ selectedCreateKeys.size > 1 ? 's' : '' }} selected to create on target
          </div>
          <!-- Select all -->
          <button
            type="button"
            class="text-xs text-surface-400 underline hover:text-surface-200 transition-colors"
            @click="selectAllSourceOnly"
          >
            Select all source-only
          </button>
          <!-- Clear -->
          <button
            type="button"
            class="flex h-7 w-7 items-center justify-center rounded-lg text-surface-500 hover:bg-surface-800 hover:text-surface-300 transition-colors"
            title="Clear selection"
            @click="clearCreateSelection"
          >
            <X class="h-4 w-4" :stroke-width="1.75" />
          </button>
          <!-- Create button -->
          <AppButton size="sm" @click="openBatchCreateDialog">
            <TableProperties class="h-3.5 w-3.5" :stroke-width="1.75" />
            Create {{ selectedCreateKeys.size > 1 ? selectedCreateKeys.size + ' Tables' : 'Table' }} on Target
          </AppButton>
        </div>
      </Transition>

      <!-- Results table -->
      <AppStateBlock v-if="!tableRows.length" kind="empty" message="No tables found in either database." />
      <AppStateBlock v-else-if="!filteredRows.length" kind="empty" message="No tables match the current filter." />
      <AppTable v-else>
        <thead>
          <tr>
            <th class="w-8" />
            <th>Table</th>
            <th>Schema</th>
            <th>Status</th>
            <th>Column diff</th>
            <th class="w-36" />
          </tr>
        </thead>
        <tbody>
          <template v-for="row in filteredRows" :key="`${row.schema}.${row.name}`">
            <tr
              :class="[
                row.status === 'both' ? 'cursor-pointer hover:bg-muted/30' : '',
                row.status === 'source_only'
                  ? isCreateSelected(row)
                    ? 'bg-brand-500/8 ring-1 ring-inset ring-brand-500/30'
                    : 'cursor-pointer hover:bg-brand-500/5'
                  : '',
              ]"
              style="transition: background 0.1s;"
              @click="row.status === 'both' ? loadInlineDiff(row) : row.status === 'source_only' && toggleCreateSelect(row)"
            >
              <!-- Expand chevron (both) / checkbox (source_only) -->
              <td class="text-center" @click.stop>
                <ChevronRight
                  v-if="row.status === 'both' && !inlineDiffCache[diffKey(row.schema, row.name)]?.expanded"
                  class="mx-auto h-4 w-4 text-muted-foreground cursor-pointer"
                  :stroke-width="1.75"
                  @click="loadInlineDiff(row)"
                />
                <ChevronDown
                  v-else-if="row.status === 'both'"
                  class="mx-auto h-4 w-4 text-brand-500 cursor-pointer"
                  :stroke-width="1.75"
                  @click="loadInlineDiff(row)"
                />
                <!-- Checkbox for source_only rows -->
                <input
                  v-else-if="row.status === 'source_only'"
                  type="checkbox"
                  class="h-4 w-4 cursor-pointer rounded border-surface-600 accent-brand-500"
                  :checked="isCreateSelected(row)"
                  @change="toggleCreateSelect(row)"
                />
              </td>
              <td class="font-medium">{{ row.name }}</td>
              <td class="font-mono text-xs text-muted-foreground">{{ row.schema }}</td>
              <td>
                <!-- Status infographic: source icon → target icon; dimmed = absent -->
                <div class="flex items-center gap-1.5">
                  <!-- Source icon -->
                  <div
                    class="flex h-6 w-6 items-center justify-center rounded-md transition-opacity"
                    :class="row.inSource
                      ? 'bg-emerald-500/10 ring-1 ring-emerald-500/40'
                      : 'bg-surface-800/40 opacity-30'"
                    :title="row.inSource ? ('In source (' + srcForm.type + ')') : 'Missing from source'"
                  >
                    <img :src="dbIconPath(srcForm.type)" :alt="srcForm.type" class="h-3.5 w-3.5 rounded object-contain" />
                  </div>
                  <!-- Connector line -->
                  <span
                    class="h-px w-3 shrink-0"
                    :class="row.status === 'both'
                      ? 'bg-gradient-to-r from-emerald-500/50 to-teal-400/50'
                      : 'bg-surface-600/50'"
                  />
                  <!-- Target icon -->
                  <div
                    class="flex h-6 w-6 items-center justify-center rounded-md transition-all duration-150"
                    :class="row.inTarget
                      ? 'bg-teal-500/10 ring-1 ring-teal-500/40'
                      : isCreateSelected(row)
                        ? 'bg-brand-500/20 ring-1 ring-brand-400/60 scale-105 opacity-100'
                        : 'bg-surface-800/40 opacity-30'"
                    :title="row.inTarget ? ('In target (' + tgtForm.type + ')') : 'Missing from target'"
                  >
                    <img :src="dbIconPath(tgtForm.type)" :alt="tgtForm.type" class="h-3.5 w-3.5 rounded object-contain" />
                  </div>
                </div>
              </td>
              <!-- Column diff summary chips -->
              <td>
                <template v-if="inlineDiffCache[diffKey(row.schema, row.name)]?.rows.length">
                  <div class="flex flex-wrap gap-1">
                    <span
                      class="rounded px-1.5 py-0.5 text-[10px] font-medium"
                      :class="hasDiff(inlineDiffCache[diffKey(row.schema, row.name)].rows)
                        ? 'bg-yellow-500/15 text-yellow-700 dark:text-yellow-400'
                        : 'bg-emerald-500/15 text-emerald-700 dark:text-emerald-400'"
                    >
                      {{ hasDiff(inlineDiffCache[diffKey(row.schema, row.name)].rows) ? 'Has diffs' : 'Match' }}
                    </span>
                    <span
                      v-if="inlineSummary(inlineDiffCache[diffKey(row.schema, row.name)].rows).type_diff"
                      class="rounded bg-yellow-500/10 px-1.5 py-0.5 text-[10px] text-yellow-700 dark:text-yellow-400"
                    >
                      {{ inlineSummary(inlineDiffCache[diffKey(row.schema, row.name)].rows).type_diff }} type
                    </span>
                    <span
                      v-if="inlineSummary(inlineDiffCache[diffKey(row.schema, row.name)].rows).source_only"
                      class="rounded bg-red-500/10 px-1.5 py-0.5 text-[10px] text-red-700 dark:text-red-400"
                    >
                      {{ inlineSummary(inlineDiffCache[diffKey(row.schema, row.name)].rows).source_only }} missing
                    </span>
                    <span
                      v-if="inlineSummary(inlineDiffCache[diffKey(row.schema, row.name)].rows).target_only"
                      class="rounded bg-blue-500/10 px-1.5 py-0.5 text-[10px] text-blue-700 dark:text-blue-400"
                    >
                      {{ inlineSummary(inlineDiffCache[diffKey(row.schema, row.name)].rows).target_only }} extra
                    </span>
                  </div>
                </template>
                <span v-else-if="inlineDiffCache[diffKey(row.schema, row.name)]?.loading" class="text-xs text-muted-foreground">Loading…</span>
                <span v-else-if="inlineDiffCache[diffKey(row.schema, row.name)]?.error" class="text-xs text-red-500">Error</span>
                <span v-else-if="row.status === 'both'" class="text-xs text-muted-foreground">Click row to compare</span>
              </td>
              <!-- Details / Create on Target / Drop from Target / Copy Data / View Data -->
              <td @click.stop>
                <div v-if="row.status === 'both'" class="flex items-center gap-1.5">
                  <AppButton size="sm" @click="openFullDiff(row)">
                    <ArrowLeftRight class="h-3.5 w-3.5" :stroke-width="1.75" />
                    Details
                  </AppButton>
                  <!-- View data -->
                  <button
                    type="button"
                    class="flex h-8 w-8 items-center justify-center rounded-lg text-surface-500 transition-colors hover:bg-brand-500/10 hover:text-brand-400"
                    title="View table data"
                    @click="openViewDialog(row)"
                  >
                    <Eye class="h-3.5 w-3.5" :stroke-width="1.75" />
                  </button>
                  <button
                    type="button"
                    class="flex h-8 w-8 items-center justify-center rounded-lg text-surface-500 transition-colors hover:bg-brand-500/10 hover:text-brand-400"
                    title="Copy data from source to target"
                    @click="openCopyDialog(row.schema, row.name)"
                  >
                    <Copy class="h-3.5 w-3.5" :stroke-width="1.75" />
                  </button>
                  <button
                    type="button"
                    class="flex h-8 w-8 items-center justify-center rounded-lg text-surface-500 transition-colors hover:bg-red-500/10 hover:text-red-400"
                    title="Drop table from target"
                    @click="openDropTableDialog(row.schema, row.name)"
                  >
                    <Trash2 class="h-3.5 w-3.5" :stroke-width="1.75" />
                  </button>
                </div>
                <div v-else-if="row.status === 'source_only'" class="flex items-center gap-1.5">
                  <AppButton
                    size="sm"
                    :variant="isCreateSelected(row) ? 'primary' : 'secondary'"
                    class="whitespace-nowrap"
                    @click.stop="openCreateTableDialog(row.schema, row.name)"
                  >
                    <TableProperties class="h-3.5 w-3.5" :stroke-width="1.75" />
                    Create on Target
                  </AppButton>
                  <!-- View source data -->
                  <button
                    type="button"
                    class="flex h-8 w-8 items-center justify-center rounded-lg text-surface-500 transition-colors hover:bg-brand-500/10 hover:text-brand-400"
                    title="View source table data"
                    @click="openViewDialog(row)"
                  >
                    <Eye class="h-3.5 w-3.5" :stroke-width="1.75" />
                  </button>
                  <button
                    type="button"
                    class="flex h-8 w-8 items-center justify-center rounded-lg text-surface-500 transition-colors hover:bg-brand-500/10 hover:text-brand-400"
                    title="Copy data — create target table first"
                    @click="openCopyDialog(row.schema, row.name)"
                  >
                    <Copy class="h-3.5 w-3.5" :stroke-width="1.75" />
                  </button>
                </div>
                <div v-else-if="row.status === 'target_only'" class="flex items-center gap-1.5">
                  <AppButton
                    size="sm"
                    variant="destructive"
                    class="whitespace-nowrap"
                    @click="openDropTableDialog(row.schema, row.name)"
                  >
                    <Trash2 class="h-3.5 w-3.5" :stroke-width="1.75" />
                    Drop from Target
                  </AppButton>
                  <!-- View target data -->
                  <button
                    type="button"
                    class="flex h-8 w-8 items-center justify-center rounded-lg text-surface-500 transition-colors hover:bg-brand-500/10 hover:text-brand-400"
                    title="View target table data"
                    @click="openViewDialog(row)"
                  >
                    <Eye class="h-3.5 w-3.5" :stroke-width="1.75" />
                  </button>
                </div>
              </td>
            </tr>

            <!-- ── Inline column diff expansion ── -->
            <tr
              v-if="inlineDiffCache[diffKey(row.schema, row.name)]?.expanded"
              :key="`diff-${row.schema}.${row.name}`"
              class="bg-surface-900/60"
            >
              <td colspan="6" class="px-6 pb-5 pt-4">
                <AppStateBlock
                  v-if="inlineDiffCache[diffKey(row.schema, row.name)]?.loading"
                  kind="loading"
                  message="Comparing column structures…"
                />
                <p v-else-if="inlineDiffCache[diffKey(row.schema, row.name)]?.error" class="text-sm text-red-700 dark:text-red-400">
                  {{ inlineDiffCache[diffKey(row.schema, row.name)].error }}
                </p>
                <template v-else-if="inlineDiffCache[diffKey(row.schema, row.name)]?.rows.length">
                  <!-- Column header labels -->
                  <div class="mb-2 grid grid-cols-[1fr_20px_1fr] gap-x-3 px-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                    <span>Source column · type</span>
                    <span />
                    <span class="text-right">Target column · type</span>
                  </div>
                  <!-- Column rows -->
                  <div class="space-y-0.5">
                    <div
                      v-for="col in inlineDiffCache[diffKey(row.schema, row.name)].rows"
                      :key="col.name"
                      class="grid grid-cols-[1fr_20px_1fr] items-center gap-x-3"
                    >
                      <!-- Source cell -->
                      <div
                        class="flex items-center justify-between rounded-l-md px-2.5 py-1.5 text-xs"
                        :class="{
                          'bg-emerald-500/5': col.status === 'match',
                          'bg-yellow-500/8': col.status === 'type_diff',
                          'bg-red-500/8': col.status === 'source_only',
                          'bg-transparent': col.status === 'target_only',
                        }"
                      >
                        <span
                          v-if="col.sourceCol"
                          class="font-mono font-medium truncate"
                          :class="col.status === 'source_only' ? 'text-red-600 dark:text-red-400' : ''"
                        >{{ col.sourceCol.name }}</span>
                        <span v-else class="text-muted-foreground">—</span>
                        <span
                          v-if="col.sourceCol"
                          class="ml-2 shrink-0 font-mono text-[10px]"
                          :class="col.status === 'type_diff' ? 'font-semibold text-yellow-600 dark:text-yellow-400' : 'text-muted-foreground'"
                        >{{ col.sourceCol.data_type }}</span>
                      </div>
                      <!-- Status dot -->
                      <div class="flex items-center justify-center">
                        <span
                          class="h-2 w-2 rounded-full"
                          :class="{
                            'bg-emerald-500': col.status === 'match',
                            'bg-yellow-400': col.status === 'type_diff',
                            'bg-red-500': col.status === 'source_only',
                            'bg-blue-500': col.status === 'target_only',
                          }"
                        />
                      </div>
                      <!-- Target cell -->
                      <div
                        class="flex items-center justify-between rounded-r-md px-2.5 py-1.5 text-xs"
                        :class="{
                          'bg-emerald-500/5': col.status === 'match',
                          'bg-yellow-500/8': col.status === 'type_diff',
                          'bg-transparent': col.status === 'source_only',
                          'bg-blue-500/8': col.status === 'target_only',
                        }"
                      >
                        <span
                          v-if="col.targetCol"
                          class="font-mono font-medium truncate"
                          :class="col.status === 'target_only' ? 'text-blue-600 dark:text-blue-400' : ''"
                        >{{ col.targetCol.name }}</span>
                        <span v-else class="text-muted-foreground">—</span>
                        <span
                          v-if="col.targetCol"
                          class="ml-2 shrink-0 font-mono text-[10px]"
                          :class="col.status === 'type_diff' ? 'font-semibold text-yellow-600 dark:text-yellow-400' : 'text-muted-foreground'"
                        >{{ col.targetCol.data_type }}</span>
                      </div>
                    </div>
                  </div>
                </template>
              </td>
            </tr>
          </template>
        </tbody>
      </AppTable>
    </template>

      <!-- ── Full column diff dialog ── -->
      <TableStructureDiffDialog
        v-if="diffOpen && diffSrcTable && diffTgtTable"
        :open="diffOpen"
        :source="buildSrcPayload()"
        :target="buildTgtPayload()"
        :source-schema="diffSrcTable.schema"
        :source-table="diffSrcTable.name"
        :target-schema="diffTgtTable.schema"
        :target-table="diffTgtTable.name"
        @update:open="(open) => { if (!open) diffOpen = false }"
      />
      <!-- ── DDL Dialog (Create Schema / Create Table / Drop Table on Target) ── -->
      <Teleport to="body">
        <div v-if="ddlDialogMode" class="modal-overlay z-50" @click.self="closeDdlDialog">
          <AppCard class="w-full max-w-lg animate-slide-up space-y-4 !p-5">
            <!-- Header -->
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-2.5">
                <div
                  class="flex h-8 w-8 items-center justify-center rounded-lg"
                  :class="ddlDialogMode === 'drop' ? 'bg-red-500/15' : 'bg-brand-500/15'"
                >
                  <component
                    :is="ddlDialogMode === 'schema' ? FolderPlus : ddlDialogMode === 'drop' ? Trash2 : TableProperties"
                    class="h-4 w-4"
                    :class="ddlDialogMode === 'drop' ? 'text-red-400' : 'text-brand-400'"
                    :stroke-width="1.75"
                  />
                </div>
                <div>
                  <p class="text-sm font-semibold">
                    {{ ddlDialogMode === 'schema' ? 'Create Schema on Target' : ddlDialogMode === 'drop' ? 'Drop Table from Target' : 'Create Table on Target' }}
                  </p>
                  <p class="text-xs text-surface-400">
                    <template v-if="ddlDialogMode === 'schema'">Creates a new schema in the target database.</template>
                    <template v-else-if="ddlDialogMode === 'drop'">Permanently removes the table from the target database.</template>
                    <template v-else>Clones "{{ ddlPendingTable?.schema }}"."{{ ddlPendingTable?.name }}" from source to target.</template>
                  </p>
                </div>
              </div>
              <button type="button" class="text-surface-400 hover:text-foreground" @click="closeDdlDialog">
                <X class="h-4 w-4" :stroke-width="1.75" />
              </button>
            </div>

            <!-- Drop Table warning + DDL preview -->
            <template v-if="ddlDialogMode === 'drop' && ddlPendingTable">
              <div class="flex items-start gap-3 rounded-xl border border-red-500/20 bg-red-500/5 p-3">
                <AlertTriangle class="mt-0.5 h-4 w-4 shrink-0 text-red-400" :stroke-width="1.75" />
                <div class="text-xs text-red-700 dark:text-red-400">
                  <p class="font-semibold">Destructive operation — cannot be undone.</p>
                  <p class="mt-0.5 text-red-500">All data in this table will be permanently deleted from the target database.</p>
                </div>
              </div>
              <div>
                <span class="auth-label">DDL statement</span>
                <div class="mt-1 rounded-lg border border-red-500/20 bg-surface-950 p-3">
                  <pre class="text-[11px] text-red-400">{{ buildDropTableDdl(ddlPendingTable.schema, ddlPendingTable.name) }}</pre>
                </div>
              </div>
            </template>

            <!-- Schema name input (for schema creation and table target schema override) -->
            <label v-if="ddlDialogMode !== 'drop'">
              <span class="auth-label">{{ ddlDialogMode === 'schema' ? 'Schema name' : 'Target schema' }}</span>
              <AppInput
                v-model="ddlNewSchema"
                :placeholder="ddlDialogMode === 'schema' ? 'e.g. bookings' : ddlPendingTable?.schema ?? 'same as source'"
                autofocus
              />
              <p v-if="ddlDialogMode === 'table'" class="mt-1 text-[11px] text-surface-500">
                Leave blank to use the source schema name.
              </p>
            </label>

            <!-- DDL preview for table creation -->
            <div v-if="ddlDialogMode === 'table' && ddlPendingTable">
              <span class="auth-label">DDL preview</span>
              <!-- Loading -->
              <div v-if="inlineDiffCache[diffKey(ddlPendingTable.schema, ddlPendingTable.name)]?.loading" class="mt-1 flex items-center gap-2 rounded-lg border border-surface-700/40 bg-surface-900/60 p-3">
                <Loader2 class="h-4 w-4 animate-spin text-brand-400 shrink-0" :stroke-width="1.75" />
                <span class="text-xs text-surface-400">Loading column structure…</span>
              </div>
              <!-- Error -->
              <div v-else-if="inlineDiffCache[diffKey(ddlPendingTable.schema, ddlPendingTable.name)]?.error" class="mt-1 rounded-lg border border-red-500/20 bg-red-500/5 p-3">
                <p class="text-xs text-red-700 dark:text-red-400">{{ inlineDiffCache[diffKey(ddlPendingTable.schema, ddlPendingTable.name)].error }}</p>
              </div>
              <!-- DDL ready -->
              <div v-else-if="inlineDiffCache[diffKey(ddlPendingTable.schema, ddlPendingTable.name)]?.rows?.length" class="mt-1 rounded-lg border border-surface-700/40 bg-surface-950 p-3">
                <pre class="overflow-x-auto text-[11px] text-surface-300 whitespace-pre-wrap">{{ buildCreateTableDdl(ddlPendingTable.schema, ddlPendingTable.name) }}</pre>
              </div>
            </div>

            <!-- DDL preview for schema creation -->
            <div v-if="ddlDialogMode === 'schema' && ddlNewSchema.trim()">
              <span class="auth-label">DDL preview</span>
              <div class="mt-1 rounded-lg border border-surface-700/40 bg-surface-950 p-3">
                <pre class="text-[11px] text-surface-300">{{ buildCreateSchemaDdl() }}</pre>
              </div>
            </div>

            <p v-if="ddlError" class="auth-error">{{ ddlError }}</p>
            <p v-if="ddlSuccess && ddlCombinedPhase === 'idle'" class="rounded-xl border border-emerald-500/20 bg-emerald-500/5 p-2 text-xs text-emerald-600 dark:text-emerald-400">
              {{ ddlSuccess }}
            </p>

            <!-- ── Combined create + copy progress ── -->
            <template v-if="ddlCombinedPhase !== 'idle'">
              <!-- Step 1: DDL -->
              <div class="flex items-center gap-3 rounded-xl border border-surface-700/40 bg-surface-900/60 px-4 py-3">
                <div class="flex h-6 w-6 shrink-0 items-center justify-center">
                  <Loader2 v-if="ddlCombinedPhase === 'ddl'" class="h-4 w-4 animate-spin text-brand-400" :stroke-width="1.75" />
                  <CheckCircle2 v-else class="h-4 w-4 text-emerald-400" :stroke-width="1.75" />
                </div>
                <div class="flex-1 text-sm">
                  <span class="font-medium">Create table structure</span>
                  <span v-if="ddlSuccess" class="ml-2 text-xs text-emerald-500">{{ ddlSuccess }}</span>
                </div>
              </div>
              <!-- Step 2: Copy -->
              <div
                class="flex items-center gap-3 rounded-xl border px-4 py-3 transition-colors"
                :class="ddlCombinedPhase === 'ddl'
                  ? 'border-surface-700/30 bg-surface-900/30 opacity-50'
                  : 'border-surface-700/40 bg-surface-900/60'"
              >
                <div class="flex h-6 w-6 shrink-0 items-center justify-center">
                  <Loader2 v-if="ddlCopyRunning" class="h-4 w-4 animate-spin text-brand-400" :stroke-width="1.75" />
                  <CheckCircle2 v-else-if="ddlCombinedPhase === 'done' && !ddlCopyError" class="h-4 w-4 text-emerald-400" :stroke-width="1.75" />
                  <AlertCircle v-else-if="ddlCopyError" class="h-4 w-4 text-red-400" :stroke-width="1.75" />
                  <div v-else class="h-4 w-4 rounded-full border-2 border-surface-600" />
                </div>
                <div class="flex-1 text-sm">
                  <span class="font-medium">Copy all rows from source</span>
                  <span v-if="ddlCopyRows !== null" class="ml-2 text-xs text-emerald-500">{{ ddlCopyRows.toLocaleString() }} rows copied</span>
                  <span v-if="ddlCopyError" class="ml-2 text-xs text-red-400">{{ ddlCopyError }}</span>
                </div>
              </div>
            </template>

            <div class="flex justify-end gap-2">
              <AppButton variant="secondary" @click="closeDdlDialog">
                {{ ddlCombinedPhase === 'done' ? 'Close' : 'Cancel' }}
              </AppButton>
              <!-- Drop execute -->
              <AppButton
                v-if="ddlDialogMode === 'drop'"
                variant="destructive"
                :disabled="ddlRunning"
                @click="execDdl(buildDropTableDdl(ddlPendingTable!.schema, ddlPendingTable!.name))"
              >
                <Loader2 v-if="ddlRunning" class="h-4 w-4 animate-spin" :stroke-width="1.75" />
                <Trash2 v-else class="h-4 w-4" :stroke-width="1.75" />
                {{ ddlRunning ? 'Dropping…' : 'Drop Table' }}
              </AppButton>
              <!-- Create only -->
              <AppButton
                v-else-if="ddlDialogMode === 'table' && ddlCombinedPhase === 'idle'"
                variant="secondary"
                :disabled="ddlRunning
                  || !inlineDiffCache[diffKey(ddlPendingTable?.schema ?? '', ddlPendingTable?.name ?? '')]?.rows?.length
                  || inlineDiffCache[diffKey(ddlPendingTable?.schema ?? '', ddlPendingTable?.name ?? '')]?.loading"
                @click="execDdl(buildCreateTableDdl(ddlPendingTable!.schema, ddlPendingTable!.name))"
              >
                <TableProperties class="h-4 w-4" :stroke-width="1.75" />
                Structure only
              </AppButton>
              <!-- Create + Copy (primary for table mode) -->
              <AppButton
                v-if="ddlDialogMode === 'table' && ddlCombinedPhase === 'idle'"
                variant="primary"
                :disabled="ddlRunning
                  || !inlineDiffCache[diffKey(ddlPendingTable?.schema ?? '', ddlPendingTable?.name ?? '')]?.rows?.length
                  || inlineDiffCache[diffKey(ddlPendingTable?.schema ?? '', ddlPendingTable?.name ?? '')]?.loading"
                @click="execDdlAndCopy"
              >
                <Copy class="h-4 w-4" :stroke-width="1.75" />
                Create &amp; Copy Data
              </AppButton>
              <!-- Schema creation -->
              <AppButton
                v-else-if="ddlDialogMode === 'schema'"
                variant="primary"
                :disabled="ddlRunning || !ddlNewSchema.trim()"
                @click="execDdl(buildCreateSchemaDdl())"
              >
                <Loader2 v-if="ddlRunning" class="h-4 w-4 animate-spin" :stroke-width="1.75" />
                <FolderPlus v-else class="h-4 w-4" :stroke-width="1.75" />
                {{ ddlRunning ? 'Executing…' : 'Create Schema' }}
              </AppButton>
            </div>
          </AppCard>
        </div>
      </Teleport>
      <!-- ── Copy Data Dialog ── -->
      <Teleport to="body">
        <div v-if="copyDialogOpen" class="modal-overlay z-50" @click.self="closeCopyDialog">
          <AppCard class="w-full max-w-lg animate-slide-up space-y-4 !p-5">

            <!-- Header -->
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-2.5">
                <div class="flex h-8 w-8 items-center justify-center rounded-lg bg-brand-500/15">
                  <Copy class="h-4 w-4 text-brand-400" :stroke-width="1.75" />
                </div>
                <div>
                  <p class="text-sm font-semibold">Copy Data to Target</p>
                  <p class="text-xs text-surface-400">
                    Migrate rows from source table to matching target table.
                  </p>
                </div>
              </div>
              <button type="button" class="text-surface-400 hover:text-foreground" @click="closeCopyDialog">
                <X class="h-4 w-4" :stroke-width="1.75" />
              </button>
            </div>

            <!-- Table being copied -->
            <div v-if="copyTable" class="flex items-center gap-3 rounded-xl border border-surface-700/50 bg-surface-900/60 px-4 py-3">
              <!-- Source side -->
              <div class="flex min-w-0 flex-1 items-center gap-2">
                <span class="h-2 w-2 shrink-0 rounded-full bg-emerald-500"></span>
                <div class="min-w-0">
                  <p class="text-[10px] font-semibold uppercase tracking-wide text-emerald-500/80">Source</p>
                  <p class="truncate font-mono text-xs text-surface-200">
                    {{ copyTable.schema }}.{{ copyTable.name }}
                  </p>
                </div>
              </div>
              <ArrowRight class="h-4 w-4 shrink-0 text-surface-500" :stroke-width="1.5" />
              <!-- Target side -->
              <div class="flex min-w-0 flex-1 items-center gap-2">
                <span class="h-2 w-2 shrink-0 rounded-full bg-teal-400"></span>
                <div class="min-w-0">
                  <p class="text-[10px] font-semibold uppercase tracking-wide text-teal-400/80">Target</p>
                  <p class="truncate font-mono text-xs text-surface-200">
                    {{ copyTable.schema }}.{{ copyTable.name }}
                  </p>
                </div>
              </div>
            </div>

            <!-- Options -->
            <div class="space-y-3">
              <!-- Truncate toggle -->
              <label class="flex items-start gap-3 rounded-xl border border-surface-700/40 bg-surface-900/40 px-4 py-3 cursor-pointer hover:border-surface-600/60 transition-colors">
                <input
                  v-model="copyTruncate"
                  type="checkbox"
                  class="mt-0.5 h-4 w-4 rounded accent-brand-500"
                />
                <div>
                  <p class="text-sm font-medium">Truncate target before copy</p>
                  <p class="text-xs text-surface-400">Deletes all existing rows in the target table before inserting. Use when performing a full refresh.</p>
                </div>
              </label>

              <!-- Batch size -->
              <label class="flex items-center justify-between gap-4">
                <div>
                  <p class="text-sm font-medium">Batch size</p>
                  <p class="text-xs text-surface-400">Rows per INSERT (1–5000)</p>
                </div>
                <AppInput
                  v-model="copyBatchSize"
                  type="number"
                  min="1"
                  max="5000"
                  class="w-28 text-right"
                />
              </label>
            </div>

            <!-- Force direct copy toggle -->
            <label
              class="flex cursor-pointer items-start gap-3 rounded-xl border px-4 py-3 transition-colors"
              :class="copyForce
                ? 'border-brand-500/40 bg-brand-500/5'
                : 'border-yellow-500/20 bg-yellow-500/5 hover:border-yellow-500/40'"
            >
              <input
                v-model="copyForce"
                type="checkbox"
                class="mt-0.5 h-4 w-4 shrink-0 rounded accent-brand-500"
              />
              <div>
                <p class="text-sm font-medium" :class="copyForce ? 'text-brand-400' : 'text-yellow-600 dark:text-yellow-400'">
                  Force direct copy for large datasets
                </p>
                <p class="mt-0.5 text-xs" :class="copyForce ? 'text-surface-400' : 'text-yellow-700/80 dark:text-yellow-500/80'">
                  <template v-if="copyForce">
                    Timeout extended to <strong>30 minutes</strong>. Direct SQL copy will proceed regardless of table size.
                  </template>
                  <template v-else>
                    Default timeout is <strong>5 minutes</strong>. For very large tables, check this box or use a
                    <strong>CDC pipeline with SyncOnce</strong> capture mode for reliable bulk transfer.
                  </template>
                </p>
              </div>
            </label>

            <!-- Results -->
            <template v-if="copyResults.length">
              <div class="space-y-1.5">
                <p class="auth-label">Result</p>
                <div
                  v-for="r in copyResults"
                  :key="`${r.schema}.${r.name}`"
                  class="flex items-center gap-3 rounded-lg px-3 py-2.5"
                  :class="r.status === 'ok' ? 'bg-emerald-500/5 border border-emerald-500/20' : 'bg-red-500/5 border border-red-500/20'"
                >
                  <CheckCircle2 v-if="r.status === 'ok'" class="h-4 w-4 shrink-0 text-emerald-500" :stroke-width="1.75" />
                  <AlertCircle v-else class="h-4 w-4 shrink-0 text-red-400" :stroke-width="1.75" />
                  <div class="min-w-0 flex-1">
                    <p class="font-mono text-xs font-medium text-surface-200">{{ r.schema }}.{{ r.name }}</p>
                    <p v-if="r.status === 'ok'" class="text-xs text-emerald-600 dark:text-emerald-400">
                      {{ r.rows_copied.toLocaleString() }} row{{ r.rows_copied !== 1 ? 's' : '' }} copied
                    </p>
                    <p v-else class="text-xs text-red-500">{{ r.error }}</p>
                  </div>
                </div>
              </div>
            </template>

            <p v-if="copyError && !copyResults.length" class="auth-error">{{ copyError }}</p>

            <div class="flex justify-end gap-2">
              <AppButton variant="secondary" @click="closeCopyDialog">
                {{ copyResults.length ? 'Close' : 'Cancel' }}
              </AppButton>
              <AppButton
                v-if="!copyResults.length || copyError"
                variant="primary"
                :disabled="copyRunning"
                @click="runCopyData"
              >
                <Loader2 v-if="copyRunning" class="h-4 w-4 animate-spin" :stroke-width="1.75" />
                <Copy v-else class="h-4 w-4" :stroke-width="1.75" />
                {{ copyRunning ? 'Copying…' : 'Copy Data' }}
              </AppButton>
            </div>

          </AppCard>
        </div>
      </Teleport>

      <!-- ── View Table Data Dialog ── -->
      <Teleport to="body">
        <div v-if="viewDialogOpen" class="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60" @click.self="closeViewDialog">
          <div class="flex h-[90vh] w-[95vw] max-w-7xl flex-col rounded-2xl border border-surface-700 bg-surface-900 shadow-2xl ring-1 ring-black/10 animate-slide-up">

            <!-- Header -->
            <div class="flex shrink-0 items-center justify-between border-b border-surface-700/40 px-5 py-3.5">
              <div class="flex items-center gap-3">
                <div class="flex h-8 w-8 items-center justify-center rounded-lg bg-brand-500/15">
                  <Eye class="h-4 w-4 text-brand-400" :stroke-width="1.75" />
                </div>
                <div>
                  <p class="text-sm font-semibold">
                    {{ viewTable?.schema }}.{{ viewTable?.name }}
                  </p>
                  <p class="text-xs text-surface-400">Live table data</p>
                </div>
              </div>
              <button type="button" class="text-surface-400 hover:text-foreground" @click="closeViewDialog">
                <X class="h-4 w-4" :stroke-width="1.75" />
              </button>
            </div>

            <!-- Toolbar: tabs + search -->
            <div class="flex shrink-0 flex-wrap items-center gap-3 border-b border-surface-700/40 px-5 py-3">
              <!-- Source / Target tabs -->
              <div v-if="viewAvailableSides.length > 1" class="flex rounded-lg border border-surface-700/50 bg-surface-950/60 p-0.5">
                <button
                  v-for="side in viewAvailableSides"
                  :key="side"
                  type="button"
                  class="rounded-md px-3 py-1 text-xs font-medium transition-colors"
                  :class="viewSide === side
                    ? side === 'source'
                      ? 'bg-emerald-500/20 text-emerald-400'
                      : 'bg-teal-500/20 text-teal-400'
                    : 'text-surface-400 hover:text-surface-200'"
                  @click="onViewSideChange(side)"
                >
                  <span class="flex items-center gap-1.5">
                    <img
                      :src="dbIconPath(side === 'source' ? srcForm.type : tgtForm.type)"
                      :alt="side === 'source' ? srcForm.type : tgtForm.type"
                      class="h-4 w-4 rounded object-contain"
                    />
                    {{ side === 'source' ? 'Source' : 'Target' }}
                  </span>
                </button>
              </div>
              <!-- Side indicator (single side) -->
              <div v-else class="flex items-center gap-1.5 rounded-lg border border-surface-700/50 px-3 py-1.5 text-xs font-medium">
                <img
                  :src="dbIconPath(viewSide === 'source' ? srcForm.type : tgtForm.type)"
                  :alt="viewSide === 'source' ? srcForm.type : tgtForm.type"
                  class="h-4 w-4 rounded object-contain"
                />
                <span :class="viewSide === 'source' ? 'text-emerald-400' : 'text-teal-400'">
                  {{ viewSide === 'source' ? 'Source' : 'Target' }}
                </span>
              </div>

              <!-- Search -->
              <div class="relative flex-1 min-w-[200px] max-w-xs">
                <Search class="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-surface-500" :stroke-width="1.75" />
                <input
                  :value="viewSearchInput"
                  type="text"
                  placeholder="Search across all columns…"
                  class="auth-input w-full pl-8 py-1.5 text-xs"
                  @input="onViewSearchInput(($event.target as HTMLInputElement).value)"
                />
              </div>

              <!-- Row/page info -->
              <div class="ml-auto text-xs text-surface-500">
                <template v-if="viewRows.length">
                  Rows {{ viewRowStart }}–{{ viewRowEnd }}
                  <span v-if="viewHasMore"> · more available</span>
                </template>
                <template v-else-if="!viewLoading && !viewError">No rows</template>
              </div>
            </div>

            <!-- Data table (scrollable) -->
            <div class="min-h-0 flex-1 overflow-auto">
              <!-- Loading -->
              <div v-if="viewLoading" class="flex h-full items-center justify-center gap-2 text-surface-400">
                <Loader2 class="h-5 w-5 animate-spin text-brand-400" :stroke-width="1.75" />
                <span class="text-sm">Loading…</span>
              </div>
              <!-- Error -->
              <div v-else-if="viewError" class="flex h-full items-center justify-center p-8">
                <p class="max-w-md text-center text-sm text-red-400">{{ viewError }}</p>
              </div>
              <!-- Empty -->
              <div v-else-if="!viewRows.length" class="flex h-full items-center justify-center p-8">
                <p class="text-sm text-surface-500">
                  {{ viewSearch ? 'No rows match the search.' : 'This table has no rows.' }}
                </p>
              </div>
              <!-- Table -->
              <table v-else class="w-full min-w-max border-separate border-spacing-0 text-xs">
                <thead class="sticky top-0 z-10">
                  <tr>
                    <th
                      class="whitespace-nowrap border-b border-r border-surface-700/40 bg-surface-900 px-3 py-2 text-left text-[10px] font-semibold uppercase tracking-wide text-surface-500"
                    >
                      #
                    </th>
                    <th
                      v-for="col in viewColumns"
                      :key="col"
                      class="whitespace-nowrap border-b border-r border-surface-700/40 bg-surface-900 px-3 py-2 text-left text-[10px] font-semibold uppercase tracking-wide text-surface-300"
                    >
                      {{ col }}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  <tr
                    v-for="(row, ri) in viewRows"
                    :key="ri"
                    class="transition-colors hover:bg-surface-800/40"
                  >
                    <!-- Row number -->
                    <td class="border-b border-r border-surface-700/20 px-3 py-1.5 font-mono text-[10px] text-surface-600">
                      {{ viewRowStart + ri }}
                    </td>
                    <!-- Cells -->
                    <td
                      v-for="(cell, ci) in row"
                      :key="ci"
                      class="max-w-[280px] truncate border-b border-r border-surface-700/20 px-3 py-1.5"
                      :title="isNullCell(cell) ? 'NULL' : formatCell(cell)"
                    >
                      <span v-if="isNullCell(cell)" class="italic text-surface-600">NULL</span>
                      <span v-else class="font-mono">{{ formatCell(cell) }}</span>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>

            <!-- Pagination footer -->
            <div class="flex shrink-0 items-center justify-between border-t border-surface-700/40 px-5 py-2.5">
              <span class="text-xs text-surface-500">
                {{ viewColumns.length }} column{{ viewColumns.length !== 1 ? 's' : '' }}
              </span>
              <div class="flex items-center gap-2">
                <button
                  type="button"
                  class="flex h-7 w-7 items-center justify-center rounded-lg text-surface-400 transition-colors disabled:opacity-30 hover:bg-surface-800 hover:text-surface-200"
                  :disabled="viewPage === 0 || viewLoading"
                  @click="viewPrevPage"
                >
                  <ChevronLeft class="h-4 w-4" :stroke-width="1.75" />
                </button>
                <span class="min-w-[60px] text-center text-xs text-surface-400">
                  Page {{ viewPage + 1 }}
                </span>
                <button
                  type="button"
                  class="flex h-7 w-7 items-center justify-center rounded-lg text-surface-400 transition-colors disabled:opacity-30 hover:bg-surface-800 hover:text-surface-200"
                  :disabled="!viewHasMore || viewLoading"
                  @click="viewNextPage"
                >
                  <ChevronRight class="h-4 w-4" :stroke-width="1.75" />
                </button>
              </div>
              <button
                type="button"
                class="text-xs text-surface-400 hover:text-surface-200 transition-colors"
                @click="closeViewDialog"
              >
                Close
              </button>
            </div>

          </div>
        </div>
      </Teleport>

      <!-- ── Batch Create Dialog ── -->
      <Teleport v-if="batchCreateOpen" to="body">
        <div class="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60">
          <div class="flex w-full max-w-lg flex-col gap-0 rounded-2xl border border-surface-700 bg-surface-900 shadow-2xl ring-1 ring-black/10 overflow-hidden">

            <!-- Header -->
            <div class="flex items-center justify-between border-b border-surface-700/60 px-5 py-4">
              <div class="flex items-center gap-3">
                <div class="flex h-8 w-8 items-center justify-center rounded-lg bg-brand-500/15">
                  <TableProperties class="h-4 w-4 text-brand-400" :stroke-width="1.75" />
                </div>
                <div>
                  <div class="text-sm font-semibold">Create Tables on Target</div>
                  <div class="text-xs text-surface-400">
                    {{ batchCreateItems.length }} table{{ batchCreateItems.length > 1 ? 's' : '' }} ·
                    <img :src="dbIconPath(tgtForm.type)" :alt="tgtForm.type" class="inline h-3.5 w-3.5 rounded object-contain" />
                    {{ tgtForm.type }} · {{ tgtForm.database }}
                  </div>
                </div>
              </div>
              <button
                type="button"
                :disabled="batchCreateRunning"
                class="flex h-8 w-8 items-center justify-center rounded-lg text-surface-400 transition-colors hover:bg-surface-800 hover:text-surface-200 disabled:opacity-40"
                @click="closeBatchCreate"
              >
                <X class="h-4 w-4" :stroke-width="1.75" />
              </button>
            </div>

            <!-- Table list -->
            <div class="max-h-72 overflow-y-auto divide-y divide-surface-800">
              <div
                v-for="item in batchCreateItems"
                :key="item.schema + '.' + item.name"
                class="flex items-center gap-3 px-5 py-3 text-sm"
              >
                <!-- Status icon -->
                <div class="flex h-6 w-6 shrink-0 items-center justify-center">
                  <Loader2 v-if="item.status === 'running'" class="h-4 w-4 animate-spin text-brand-400" :stroke-width="1.75" />
                  <CheckCircle2 v-else-if="item.status === 'done'" class="h-4 w-4 text-emerald-400" :stroke-width="1.75" />
                  <AlertCircle v-else-if="item.status === 'error'" class="h-4 w-4 text-red-400" :stroke-width="1.75" />
                  <div v-else class="h-4 w-4 rounded-full border-2 border-surface-600"></div>
                </div>
                <!-- Table name -->
                <div class="flex-1 min-w-0">
                  <span class="font-mono text-xs text-surface-400">{{ item.schema }}.</span><span class="font-medium">{{ item.name }}</span>
                </div>
                <!-- Message -->
                <div class="shrink-0 text-xs" :class="item.status === 'error' ? 'text-red-400' : 'text-emerald-400'">
                  {{ item.message }}
                </div>
              </div>
            </div>

            <!-- Footer -->
            <div class="flex items-center justify-between border-t border-surface-700/60 px-5 py-4">
              <div class="text-xs text-surface-500">
                <template v-if="batchCreateDone">
                  {{ batchCreateItems.filter(i => i.status === 'done').length }} created,
                  {{ batchCreateItems.filter(i => i.status === 'error').length }} failed
                </template>
                <template v-else-if="batchCreateRunning">
                  Creating tables one by one…
                </template>
                <template v-else>
                  Review the list, then click Create to execute DDL on target.
                </template>
              </div>
              <div class="flex items-center gap-2">
                <AppButton variant="secondary" size="sm" :disabled="batchCreateRunning" @click="closeBatchCreate">
                  {{ batchCreateDone ? 'Close' : 'Cancel' }}
                </AppButton>
                <AppButton
                  v-if="!batchCreateDone"
                  size="sm"
                  :disabled="batchCreateRunning"
                  @click="runBatchCreate"
                >
                  <Loader2 v-if="batchCreateRunning" class="h-3.5 w-3.5 animate-spin" :stroke-width="1.75" />
                  <TableProperties v-else class="h-3.5 w-3.5" :stroke-width="1.75" />
                  {{ batchCreateRunning ? 'Creating…' : 'Create All' }}
                </AppButton>
              </div>
            </div>

          </div>
        </div>
      </Teleport>
    </template>
    <!-- end workspace mode -->

  </div>
</template>
