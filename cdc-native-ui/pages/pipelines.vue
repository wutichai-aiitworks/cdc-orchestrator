<script setup lang="ts">
import type { PipelineDetailAction } from "~/components/PipelineDetail.vue"
import {
  ArrowDown,
  ArrowRight,
  ArrowUp,
  ArrowUpDown,
  CalendarClock,
  CheckCircle2,
  Eye,
  Filter,
  KeyRound,
  LayoutGrid,
  Pencil,
  Plus,
  Radar,
  RefreshCw,
  Save,
  Search,
  Table2,
  Tags,
  TriangleAlert,
  Wand2,
  Trash2,
  Waves,
  Zap,
} from "@lucide/vue"
import type { Component } from "vue"
import type {
  CdcProfile,
  ChangeMode,
  ColumnLink,
  DatabaseType,
  GovernanceConfig,
  LakeCatalogType,
  LakeZone,
  PipelinePattern,
  ProfileDlqStatus,
  ProfilePayload,
  Sensitivity,
  SinkErrorPolicy,
  SourceTableColumn,
  TableFormat,
  TableMapping,
  TablePollRule,
} from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"
import { DB_TYPE_OPTIONS, dbIconPath } from "~/utils/dbIcon"

type WizardStep = "source" | "transform" | "target" | "mapping" | "review"
type TransformPreset = "none" | "mask_pii" | "normalize_names"
type MappingMode = "identical" | "custom"
type CaptureMode = "logical" | "trigger" | "sync_once" | "incremental"

const {
  fetchProfiles,
  createProfile,
  updateProfile,
  deleteProfile,
  fetchDlqStatus,
  validateSource,
  validateTarget,
  validateLake,
  browseSourceCatalog,
  browseTargetCatalog,
  inspectSourceTable,
  inspectTargetTable,
  installSourceTriggers,
  sourceTriggerStatus,
  setEnabled,
  runAction,
  resyncProfile,
} = useProfilesM3()
const toast = useToast()

const profiles = ref<CdcProfile[]>([])
const selectedId = ref("")
const editing = ref(false)
const step = ref<WizardStep>("source")
const loading = ref(false)
const saving = ref(false)
const deleting = ref(false)
const pendingDelete = ref<CdcProfile | null>(null)
const validating = ref(false)
const error = ref("")
const sourceValidation = ref<{ valid: boolean; message: string } | null>(null)
const targetValidation = ref<{ valid: boolean; message: string } | null>(null)
const dlqStatus = ref<ProfileDlqStatus | null>(null)
const dlqLoading = ref(false)
const dlqError = ref("")
/** True when a password is already stored in the saved profile (not re-entered yet). */
const passwordStored = ref(false)
const targetPasswordStored = ref(false)

const dbTypes = DB_TYPE_OPTIONS

const form = reactive({
  name: "",
  description: "",
  tags: [] as string[],
  pattern: "source_to_target" as PipelinePattern,
  sourceType: "postgres" as DatabaseType,
  host: "",
  port: "5432",
  database: "",
  username: "",
  password: "",
  schemaMode: "specific_schemas" as "all_schemas" | "specific_schemas",
  tableMode: "all_tables" as "all_tables" | "specific_tables",
  schemas: "",
  tables: "",
  captureMode: "logical" as CaptureMode,
  sinkErrorPolicy: "fail_fast" as SinkErrorPolicy,
  autostart: true,
  streamPlatform: "redpanda" as "kafka" | "redpanda",
  transformPreset: "none" as TransformPreset,
  deliveryMode: "realtime" as "realtime" | "micro_batch",
  intervalSeconds: "",
  fullRefreshMinutes: "",
  changelogRetentionHours: "",
  timestampColumn: "",
  incrementingColumn: "",
  upsertKeyColumns: "",
  tablePoll: [] as TablePollRule[],
  targetName: "",
  targetType: "postgres" as DatabaseType,
  targetHost: "",
  targetPort: "5432",
  targetDatabase: "",
  targetUsername: "",
  targetPassword: "",
  mappingMode: "identical" as MappingMode,
  mappings: [] as TableMapping[],
  lakeName: "",
  lakeFormat: "iceberg" as TableFormat,
  lakeZone: "bronze" as LakeZone,
  lakeChangeMode: "changelog" as ChangeMode,
  lakeCatalog: "jdbc" as LakeCatalogType,
  lakeEndpoint: "http://cdc-lake:9000",
  lakeBucket: "lake",
  lakeRegion: "us-east-1",
  lakeAccessKey: "cdcadmin",
  lakeSecretKey: "",
  lakeWarehouse: "s3://lake/warehouse",
  lakeNamespace: "",
  lakeCatalogUri: "jdbc:postgresql://cdc-postgres-target:5432/demo",
  lakeCatalogUser: "demo",
  lakeCatalogPassword: "",
  domain: "",
  dataset: "",
  sensitivity: "internal" as Sensitivity,
  industryPack: "",
  eventLandIceberg: false,
})

const filled = (value: string | number | null | undefined) => String(value ?? "").trim().length > 0
const textValue = (value: string | number | null | undefined) => String(value ?? "").trim()

const isEventFirstPattern = (pattern: PipelinePattern) =>
  pattern === "source_to_stream" || pattern === "source_to_lake"

const forceLogicalCapture = () => {
  if (form.captureMode === "trigger" || form.captureMode === "incremental") {
    form.captureMode = "logical"
  }
}

const selectReplicaDestination = () => {
  form.pattern = "source_to_target"
  form.eventLandIceberg = false
}

const selectEventDestination = () => {
  if (!isEventFirstPattern(form.pattern)) form.eventLandIceberg = false
  form.pattern = form.eventLandIceberg ? "source_to_lake" : "source_to_stream"
  forceLogicalCapture()
}

const selectAnalyticsDestination = () => {
  form.pattern = "source_to_olake"
  form.eventLandIceberg = false
  forceLogicalCapture()
}

const setEventLandIceberg = (value: boolean) => {
  form.eventLandIceberg = value
  form.pattern = value ? "source_to_lake" : "source_to_stream"
  forceLogicalCapture()
}

const captureIdentityKey = (sourceType: string, host: string, port: string, database: string) =>
  `${sourceType.trim().toLowerCase()}|${host.trim().toLowerCase()}|${port.trim()}|${database.trim().toLowerCase()}`

const dualCaptureWarning = computed(() => {
  if (!form.host.trim() && !form.database.trim()) return ""
  const identity = captureIdentityKey(form.sourceType, form.host, form.port, form.database)
  const formIsOlake = form.pattern === "source_to_olake"
  for (const existing of profiles.value) {
    if (selectedId.value && existing.id === selectedId.value) continue
    const other = captureIdentityKey(
      existing.source.source_type,
      existing.source.connection.host || "",
      existing.source.connection.port || "",
      existing.source.connection.database || "",
    )
    if (other !== identity) continue
    const otherOlake = existing.pattern === "source_to_olake"
    if (formIsOlake !== otherOlake) {
      return `This source is already captured by “${existing.name}”. One reader per database: Analytics-first (OLake) cannot share a source with replica or Event-first (Debezium).`
    }
  }
  return ""
})

const startBlockedReason = (pattern?: string | null) => {
  if (!pattern || pattern === "source_to_target") return ""
  if (pattern === "source_to_olake") {
    return "Analytics-first cannot start yet. Save a draft; the OLake stack is Phase 3."
  }
  return "Event-first start is Phase 2. Database replica can Start now."
}

const selected = computed(() => profiles.value.find((profile) => profile.id === selectedId.value) || null)
const canStartSelected = computed(() => selected.value?.pattern === "source_to_target")

const supportsCappedTableCopy = computed(() => {
  const mode = String(selected.value?.source?.capture_mode || "").toLowerCase()
  return mode === "trigger" || mode === "incremental"
})

const LIVE_CATALOG_TYPES = ["postgres", "postgresql", "mysql", "mariadb", "sqlserver", "mssql", "sqlite", "oracle"]
const sourceLiveCatalogSupported = computed(() =>
  LIVE_CATALOG_TYPES.includes(form.sourceType.toLowerCase())
)

const canBrowseSource = computed(
  () =>
    form.sourceType === "sqlite"
      ? filled(form.database)
      : filled(form.host) &&
        filled(form.port) &&
        filled(form.database) &&
        filled(form.username) &&
        Boolean(selectedId.value || form.password)
)
const canContinueSource = computed(
  () =>
    canBrowseSource.value &&
    (form.schemaMode !== "specific_schemas" || filled(form.schemas)) &&
    (form.tableMode !== "specific_tables" || filled(form.tables)) &&
    (form.captureMode !== "trigger" || filled(form.tables)) &&
    (!isIncrementalCapture() || incrementalSourceReady()) &&
    ((form.captureMode !== "incremental" && form.captureMode !== "trigger") || form.deliveryMode !== "micro_batch" || filled(form.intervalSeconds))
)
const canContinueTarget = computed(() => {
  if (form.pattern === "source_to_stream" || form.pattern === "source_to_olake") return true
  if (form.pattern === "source_to_lake") {
    return (
      filled(form.lakeEndpoint) &&
      filled(form.lakeBucket) &&
      filled(form.lakeAccessKey) &&
      Boolean(selectedId.value || form.lakeSecretKey) &&
      (form.lakeFormat !== "iceberg" ||
        form.lakeCatalog === "hadoop" ||
        form.lakeCatalog === "glue" ||
        filled(form.lakeCatalogUri))
    )
  }
  // targetName is optional (display label only — auto-derived from host/database)
  return (
    filled(form.targetHost) &&
    filled(form.targetPort) &&
    filled(form.targetDatabase) &&
    filled(form.targetUsername) &&
    Boolean(selectedId.value || form.targetPassword)
  )
})
const canTestTarget = computed(() => {
  if (form.pattern === "source_to_stream" || form.pattern === "source_to_olake") return false
  if (form.pattern === "source_to_lake") {
    return filled(form.lakeEndpoint) && filled(form.lakeBucket) && filled(form.lakeAccessKey)
  }
  return (
    filled(form.targetHost) &&
    filled(form.targetPort) &&
    filled(form.targetDatabase) &&
    filled(form.targetUsername)
  )
})
const canLoadTargetCatalog = computed(
  () =>
    canTestTarget.value &&
    Boolean(selectedId.value || form.targetPassword || targetPasswordStored.value),
)
const syncAutofilledSecrets = () => {
  if (typeof document === "undefined") return
  const targetPassword = document.querySelector<HTMLInputElement>('input[name="targetPassword"]')
  if (targetPassword?.value && !form.targetPassword) form.targetPassword = targetPassword.value
  const lakeSecret = document.querySelector<HTMLInputElement>('input[name="lakeSecretKey"]')
  if (lakeSecret?.value && !form.lakeSecretKey) form.lakeSecretKey = lakeSecret.value
}
const mappingValid = (mapping: TableMapping) => {
  if (!filled(mapping.source_table) || !filled(mapping.target_schema) || !filled(mapping.target_table)) return false
  return mapping.columns
    .filter((column) => filled(column.source) || filled(column.target) || filled(column.data_type))
    .every((column) => filled(column.source) && filled(column.target) && filled(column.data_type))
}
const mappingKey = (schema: string, table: string) => `${schema}.${table}`.toLowerCase()
const mappingIsSameName = (mapping: TableMapping) =>
  mapping.source_schema.trim().toLowerCase() === mapping.target_schema.trim().toLowerCase() &&
  mapping.source_table.trim().toLowerCase() === mapping.target_table.trim().toLowerCase() &&
  !mapping.columns.some(
    (column) => filled(column.source) && filled(column.target) && column.source.trim() !== column.target.trim()
  )
const canContinueMapping = computed(() => {
  if (form.pattern !== "source_to_target") return true
  if (form.tableMode !== "specific_tables" || !form.tables.trim()) return true
  return form.mappings.length > 0 && form.mappings.every(mappingValid)
})

const splitList = (value: string) => value.split(",").map((item) => item.trim()).filter(Boolean)

/** Map incrementing + upsert form fields to API (comma-separated PKs → upsert_key_columns). */
const isIncrementalCapture = () => String(form.captureMode || "").toLowerCase() === "incremental"

const upsertKeysFromSource = (source: CdcProfile["source"]) => {
  const cols = source.upsert_key_columns
  if (Array.isArray(cols)) return cols.map((c) => String(c).trim()).filter(Boolean)
  return [] as string[]
}

const upsertKeysFromMappings = (mappings: TableMapping[] | undefined) => {
  const seen = new Set<string>()
  const out: string[] = []
  for (const mapping of mappings || []) {
    for (const col of mapping.pk_columns || []) {
      const key = String(col).trim()
      if (!key) continue
      const fold = key.toLowerCase()
      if (seen.has(fold)) continue
      seen.add(fold)
      out.push(key)
    }
  }
  return out
}

const upsertKeysForForm = (profile: CdcProfile) => {
  const fromSource = upsertKeysFromSource(profile.source)
  if (fromSource.length) return fromSource.join(", ")
  return upsertKeysFromMappings(profile.flow.mappings).join(", ")
}

const qualifiedTableParts = (qualified: string) => {
  const trimmed = qualified.trim()
  const dot = trimmed.indexOf(".")
  if (dot <= 0) {
    return { source_schema: splitList(form.schemas)[0] || "public", source_table: trimmed }
  }
  return { source_schema: trimmed.slice(0, dot), source_table: trimmed.slice(dot + 1) }
}

/** Persist upsert PK on flow.mappings when orchestrator build lacks source.upsert_key_columns. */
const incrementalPkStubMappings = (upsertKeys: string[]) => {
  if (!upsertKeys.length) return [] as TableMapping[]
  const stubs: TableMapping[] = []
  for (const qualified of splitList(form.tables)) {
    const { source_schema, source_table } = qualifiedTableParts(qualified)
    if (!source_table) continue
    const target_schema = source_schema
    const target_table = source_table
    stubs.push({
      source_schema,
      source_table,
      target_schema,
      target_table,
      identical: mappingIsSameName({
        source_schema,
        source_table,
        target_schema,
        target_table,
        identical: true,
        columns: [],
        pk_columns: upsertKeys,
      }),
      columns: [],
      pk_columns: [...upsertKeys],
    })
  }
  return stubs
}

const applyIncrementalSourceToForm = (profile: CdcProfile) => {
  const mode = String(profile.source.capture_mode || "").toLowerCase()
  if (mode) form.captureMode = mode as CaptureMode
  form.timestampColumn = profile.source.timestamp_column ?? ""
  form.incrementingColumn = profile.source.incrementing_column ?? ""
  form.upsertKeyColumns = upsertKeysForForm(profile)
  form.tablePoll = [...(profile.source.table_poll ?? [])]
}

/** Non-watermark poll rules for tables that are still selected (dropped tables lose their rule). */
const activeTablePoll = (): TablePollRule[] => {
  const selected = splitList(form.tables).map((t) => t.toLowerCase())
  return form.tablePoll.filter(
    (rule) => rule.strategy !== "watermark" && selected.includes(rule.table.toLowerCase()),
  )
}

/** True when every selected table has a rolling-window / full-copy rule (no watermark needed). */
const everyTableCustomPolled = () => {
  const tables = splitList(form.tables)
  if (!tables.length) return false
  const rules = activeTablePoll()
  return tables.every((t) => rules.some((r) => r.table.toLowerCase() === t.toLowerCase()))
}

/** Rolling-window rules missing their date column. */
const incompleteRollingRules = () =>
  activeTablePoll().filter((r) => r.strategy === "rolling_window" && !filled(r.window_column ?? ""))

/** Watermark columns are an incremental-only concept; other modes always post them empty. */
const buildSourceWatermarkFields = () => {
  if (!isIncrementalCapture()) {
    return {
      timestamp_column: null as string | null,
      incrementing_column: null as string | null,
      upsert_key_columns: [] as string[],
    }
  }
  const wm = incrementalWatermarkFields()
  return {
    timestamp_column: textValue(form.timestampColumn) || null,
    incrementing_column: wm.incrementing_column,
    upsert_key_columns: wm.upsert_key_columns,
  }
}

const incrementalWatermarkFields = () => {
  const raw = textValue(form.incrementingColumn)
  const upsertExtra = splitList(form.upsertKeyColumns)
  if (!raw) {
    return { incrementing_column: null as string | null, upsert_key_columns: upsertExtra }
  }
  if (raw.includes(",")) {
    const merged = [...splitList(raw), ...upsertExtra]
    const seen = new Set<string>()
    const upsert_key_columns = merged.filter((col) => {
      const key = col.toLowerCase()
      if (seen.has(key)) return false
      seen.add(key)
      return true
    })
    return { incrementing_column: null, upsert_key_columns }
  }
  return { incrementing_column: raw, upsert_key_columns: upsertExtra }
}

const incrementalSourceReady = () => {
  if (!filled(form.tables)) return false
  if (incompleteRollingRules().length) return false
  const { incrementing_column, upsert_key_columns } = incrementalWatermarkFields()
  if (everyTableCustomPolled()) {
    // No watermark poll: only the sink upsert key is needed for a replica.
    return form.pattern !== "source_to_target" || upsert_key_columns.length > 0 || Boolean(incrementing_column)
  }
  if (incrementing_column) return true
  if (filled(form.timestampColumn) && upsert_key_columns.length > 0) return true
  if (filled(form.timestampColumn) && form.pattern !== "source_to_target") return true
  return false
}

const schemaPickerHint = computed(() => {
  if (form.captureMode === "trigger") {
    return "Select tables to capture, then create a changelog trigger on each. Trigger mode requires an explicit table list."
  }
  if (isIncrementalCapture()) {
    return "Select large tables to poll. Incremental requires an explicit list. Prefer tables that share the same timestamp and PK column names."
  }
  return "Select schemas, then check source tables to capture. Leave tables unchecked to capture every table in a selected schema."
})

const incrementalSourceHint = computed(() => {
  if (!isIncrementalCapture() || incrementalSourceReady()) return ""
  if (!filled(form.tables)) return "Select at least one table for incremental capture."
  const missingColumn = incompleteRollingRules()
  if (missingColumn.length) {
    return `Set the date column for the rolling window on ${missingColumn.map((r) => r.table).join(", ")}.`
  }
  if (everyTableCustomPolled()) {
    return "Set Upsert key columns (e.g. BRANCH_NO, PRO_CODE) so the target can upsert the re-read rows."
  }
  if (form.pattern === "source_to_target") {
    if (!filled(form.timestampColumn)) {
      return "For replica: set Timestamp column, then Upsert key columns (e.g. BRANCH_NO, PRO_CODE), or a single numeric Incrementing column."
    }
    return "For replica with a timestamp: set Upsert key columns (comma-separated PK). Numeric-only tables can use Incrementing column instead."
  }
  return "Set a timestamp column and/or incrementing column."
})

type CatalogSchema = { name: string; table_count: number }
type CatalogTable = { schema: string; name: string; row_count?: number | null; primary_key?: string[] }

const catalogSchemas = ref<CatalogSchema[]>([])
const catalogTables = ref<CatalogTable[]>([])
const catalogLoading = ref(false)
const catalogError = ref("")
const targetCatalogSchemas = ref<CatalogSchema[]>([])
const targetCatalogTables = ref<CatalogTable[]>([])
const targetCatalogLoading = ref(false)
const targetCatalogError = ref("")
const targetSelectedSchemas = ref<string[]>([])
const targetTableSearch = ref("")
const triggerKeys = ref<string[]>([])
const triggerInstalling = ref("")
const triggerMessage = ref("")
const triggerError = ref("")
const triggerSql = ref("")
const triggerPreviewTables = ref<string[]>([])
const tableSearch = ref("")
const countRefreshing = ref(false)
const lastCountAt = ref<number | null>(null)
const countDeltas = ref<Record<string, number>>({})
const inspectTable = ref<{ side: "source" | "target"; schema: string; name: string } | null>(null)
const structureDiff = ref<{ source_schema: string; source_table: string; target_schema: string; target_table: string } | null>(null)
// Mapping step: generate/run CREATE TABLE scripts for target tables that do not exist yet.
const ddlTables = ref<Array<{ source_schema: string; source_table: string; target_schema: string; target_table: string }> | null>(null)
const pipelineSearch = ref("")
const pipelineStatusFilter = ref("all")
/** "all", "__untagged__", or one tag (matched case-insensitively). */
const pipelineTagFilter = ref("all")
const UNTAGGED = "__untagged__"

type PipelineViewMode = "table" | "cards" | "tags" | "capture" | "schedule"
const PIPELINE_VIEW_MODES: Array<{ key: PipelineViewMode; label: string; icon: Component }> = [
  { key: "table", label: "Table", icon: Table2 },
  { key: "cards", label: "Cards", icon: LayoutGrid },
  { key: "tags", label: "By tag", icon: Tags },
  { key: "capture", label: "By capture", icon: Radar },
  { key: "schedule", label: "Schedule", icon: CalendarClock },
]
const { getPref, setPref } = useViewPrefs()
const pipelineViewMode = ref<PipelineViewMode>("table")
onMounted(() => {
  const saved = getPref<string>("pipelines.viewMode", "table")
  if (PIPELINE_VIEW_MODES.some((m) => m.key === saved)) pipelineViewMode.value = saved as PipelineViewMode
})
watch(pipelineViewMode, (mode) => setPref("pipelines.viewMode", mode))
/** List sort: column + direction; `null` keeps the API order (newest first). */
type PipelineSortKey = "name" | "capture" | "pattern"
const pipelineSortKey = ref<PipelineSortKey | null>(null)
const pipelineSortDir = ref<"asc" | "desc">("asc")
const mappingBulkSchema = ref("")
const mappingSearch = ref("")

// Per-table column inspector state (keyed by "schema.table")
type ColInspect = { cols: SourceTableColumn[]; loading: boolean; error: string; expanded: boolean }
const colInspectCache = reactive<Record<string, ColInspect>>({})

const colKey = (schema: string, table: string) => `${schema}.${table}`

const ensureColInspect = async (
  schema: string,
  table: string,
  side: "source" | "target"
) => {
  const key = colKey(schema, table)
  if (!colInspectCache[key]) {
    colInspectCache[key] = { cols: [], loading: false, error: "", expanded: true }
  }
  const entry = colInspectCache[key]
  if (entry.cols.length || entry.loading) return
  entry.loading = true
  entry.error = ""
  try {
    const payload = buildPayload()
    const profileId = selectedId.value || undefined
    let result
    if (side === "source") {
      result = await inspectSourceTable({ source: payload.source, schema, name: table, limit: 0, profile_id: profileId })
    } else {
      const target = payload.target
      if (!target) return
      result = await inspectTargetTable({ target, schema, name: table, limit: 0, profile_id: profileId })
    }
    entry.cols = result.columns
  } catch (e: any) {
    entry.error = formatApiError(e, "Failed to load columns")
  } finally {
    entry.loading = false
  }
}

const toggleColExpand = (schema: string, table: string) => {
  const key = colKey(schema, table)
  if (!colInspectCache[key]) {
    colInspectCache[key] = { cols: [], loading: false, error: "", expanded: true }
  } else {
    colInspectCache[key].expanded = !colInspectCache[key].expanded
  }
}

const openStructureDiff = (mapping: { source_schema: string; source_table: string; target_schema: string; target_table: string }) => {
  structureDiff.value = {
    source_schema: mapping.source_schema,
    source_table: mapping.source_table,
    target_schema: mapping.target_schema || mapping.source_schema,
    target_table: mapping.target_table || mapping.source_table,
  }
}

const openColMapper = (mapping: { source_schema: string; source_table: string; target_schema: string; target_table: string }) => {
  toggleColExpand(mapping.source_schema, mapping.source_table)
  const nowExpanded = colInspectCache[colKey(mapping.source_schema, mapping.source_table)]?.expanded
  if (nowExpanded) {
    void ensureColInspect(mapping.source_schema, mapping.source_table, "source")
    if (filled(mapping.target_table) && canTestTarget.value) {
      const tSchema = mapping.target_schema || mapping.source_schema
      const tTable = mapping.target_table || mapping.source_table
      void ensureColInspect(tSchema, tTable, "target")
    }
  }
}

// Get column links for a mapping row (initialise from existing columns if needed)
const getColumnLinks = (mapping: TableMapping): ColumnLink[] => {
  if (mapping.column_links) return mapping.column_links
  // Migrate old `columns` array to column_links
  return (mapping.columns || []).map((c) => ({
    id: `lnk-${Math.random().toString(36).slice(2, 9)}`,
    sources: c.sources?.length ? c.sources : [c.source].filter(Boolean),
    target: c.target,
    fn: c.transform ?? null,
    fn_args: (c.transform_args as Record<string, unknown>) ?? {},
  }))
}

const setColumnLinks = (mapping: TableMapping, links: ColumnLink[]) => {
  mapping.column_links = links
}
let countPollTimer: ReturnType<typeof setInterval> | null = null
let deltaClearTimer: ReturnType<typeof setTimeout> | null = null
let statusPollTimer: ReturnType<typeof setInterval> | null = null

const selectedSchemaNames = computed({
  get: () => splitList(form.schemas),
  set: (names: string[]) => {
    form.schemas = names.join(", ")
  },
})
const selectedTableKeys = computed({
  get: () => splitList(form.tables),
  set: (keys: string[]) => {
    form.tables = keys.join(", ")
    form.tableMode = keys.length ? "specific_tables" : "all_tables"
  },
})

const tableSelectionHint = computed(() => {
  if (selectedTableKeys.value.length) {
    return `${selectedTableKeys.value.length} table(s) selected for capture.`
  }
  if (form.captureMode === "trigger") return "Select tables, then create a trigger on each one."
  if (isIncrementalCapture()) return "Select tables to poll on the watermark column."
  return "All tables in the selected schema(s) will be captured."
})

const schemaOptions = computed(() =>
  [...new Set(catalogSchemas.value.map((item) => item.name))].sort((a, b) => a.localeCompare(b))
)
const tablesForSelectedSchemas = computed(() => {
  const schemas = new Set(selectedSchemaNames.value)
  if (!schemas.size) return []
  const search = tableSearch.value.trim().toLowerCase()
  return catalogTables.value.filter((table) => {
    if (!schemas.has(table.schema)) return false
    if (!search) return true
    return `${table.schema}.${table.name}`.toLowerCase().includes(search)
  })
})
const tablesGrouped = computed(() => {
  const groups = new Map<string, CatalogTable[]>()
  for (const table of tablesForSelectedSchemas.value) {
    const list = groups.get(table.schema) || []
    list.push(table)
    groups.set(table.schema, list)
  }
  return [...groups.entries()].map(([schema, tables]) => ({ schema, tables }))
})
const targetSchemaOptions = computed(() =>
  [...new Set(targetCatalogSchemas.value.map((item) => item.name))].sort((a, b) => a.localeCompare(b))
)
const tablesForSelectedTargetSchemas = computed(() => {
  const schemas = new Set(targetSelectedSchemas.value)
  if (!schemas.size) return []
  const search = targetTableSearch.value.trim().toLowerCase()
  return targetCatalogTables.value.filter((table) => {
    if (!schemas.has(table.schema)) return false
    if (!search) return true
    return `${table.schema}.${table.name}`.toLowerCase().includes(search)
  })
})
const targetTablesGrouped = computed(() => {
  const groups = new Map<string, CatalogTable[]>()
  for (const table of tablesForSelectedTargetSchemas.value) {
    const list = groups.get(table.schema) || []
    list.push(table)
    groups.set(table.schema, list)
  }
  return [...groups.entries()].map(([schema, tables]) => ({ schema, tables }))
})
const qualifiedKey = (schema: string, name: string) => `${schema}.${name}`
const formatRecordCount = (count: number | null | undefined) => {
  if (count == null || Number.isNaN(Number(count))) return "—"
  const n = Math.max(0, Math.trunc(Number(count)))
  return `${n.toLocaleString()} ${n === 1 ? "record" : "records"}`
}
const countDeltaFor = (schema: string, name: string) => countDeltas.value[qualifiedKey(schema, name)] || 0
const lastCountLabel = computed(() => {
  if (!lastCountAt.value) return ""
  return `Counts updated ${new Date(lastCountAt.value).toLocaleTimeString()}`
})
const formatPrimaryKey = (keys?: string[]) => {
  if (!keys?.length) return "No PK"
  return keys.length === 1 ? `PK ${keys[0]}` : `PK ${keys.join(", ")}`
}
const isSchemaSelected = (name: string) => selectedSchemaNames.value.includes(name)
const isTableSelected = (schema: string, name: string) => {
  const key = qualifiedKey(schema, name)
  return selectedTableKeys.value.includes(key)
}
const tableCountFor = (schema: string) =>
  catalogSchemas.value.find((item) => item.name === schema)?.table_count
  ?? catalogTables.value.filter((t) => t.schema === schema).length
const targetTableCountFor = (schema: string) =>
  targetCatalogSchemas.value.find((item) => item.name === schema)?.table_count
  ?? targetCatalogTables.value.filter((t) => t.schema === schema).length
const isTargetSchemaSelected = (name: string) => targetSelectedSchemas.value.includes(name)

const toggleTargetSchema = (name: string) => {
  const selected = new Set(targetSelectedSchemas.value)
  if (selected.has(name)) selected.delete(name)
  else selected.add(name)
  targetSelectedSchemas.value = [...selected]
}

const toggleSchema = (name: string) => {
  form.schemaMode = "specific_schemas"
  const selected = new Set(selectedSchemaNames.value)
  if (selected.has(name)) {
    selected.delete(name)
    selectedTableKeys.value = selectedTableKeys.value.filter((key) => {
      const schema = key.includes(".") ? key.slice(0, key.lastIndexOf(".")) : ""
      return schema !== name
    })
  } else {
    selected.add(name)
  }
  selectedSchemaNames.value = [...selected]
}

const toggleTable = (schema: string, name: string) => {
  const key = qualifiedKey(schema, name)
  const selected = new Set(selectedTableKeys.value)
  if (isTableSelected(schema, name)) {
    for (const item of [...selected]) {
      if (item.toLowerCase() === key.toLowerCase() || item.toLowerCase() === name.toLowerCase()) {
        selected.delete(item)
      }
    }
  } else {
    selected.add(key)
    if (!isSchemaSelected(schema)) {
      selectedSchemaNames.value = [...selectedSchemaNames.value, schema]
    }
  }
  selectedTableKeys.value = [...selected]
}

const selectAllVisibleTables = () => {
  const selected = new Set(selectedTableKeys.value)
  for (const table of tablesForSelectedSchemas.value) {
    selected.add(qualifiedKey(table.schema, table.name))
  }
  selectedTableKeys.value = [...selected]
}

const clearVisibleTables = () => {
  const visible = new Set(
    tablesForSelectedSchemas.value.map((table) => qualifiedKey(table.schema, table.name).toLowerCase())
  )
  selectedTableKeys.value = selectedTableKeys.value.filter(
    (key) => !visible.has(key.toLowerCase())
  )
}

const isTriggerInstalled = (schema: string, name: string) =>
  triggerKeys.value.some((item) => item.toLowerCase() === qualifiedKey(schema, name).toLowerCase())

const applyTriggerStatus = (tables: Array<{ schema: string; name: string; installed: boolean }>) => {
  triggerKeys.value = tables
    .filter((table) => table.installed)
    .map((table) => qualifiedKey(table.schema, table.name))
}

const loadTriggerStatus = async () => {
  if (form.captureMode !== "trigger" || !canBrowseSource.value) return
  try {
    const result = await sourceTriggerStatus({
      source: buildPayload().source,
      tables: selectedTableKeys.value,
      profile_id: selectedId.value || undefined,
    })
    applyTriggerStatus(result.tables || [])
  } catch {
    /* status is optional until install */
  }
}

const requestTriggerPreview = (tables: string[]) => {
  if (form.captureMode !== "trigger") return
  const qualified = [...new Set(tables.map((item) => item.trim()).filter(Boolean))]
  if (!qualified.length) {
    triggerError.value = "Select at least one table, then create its trigger."
    return
  }
  triggerError.value = ""
  triggerMessage.value = ""
  triggerPreviewTables.value = qualified
}

const installTriggers = async (tables: string[]) => {
  if (form.captureMode !== "trigger") return
  const qualified = [...new Set(tables.map((item) => item.trim()).filter(Boolean))]
  if (!qualified.length) {
    triggerError.value = "Select at least one table, then create its trigger."
    return
  }
  triggerInstalling.value = qualified.length === 1 ? qualified[0] : "*"
  triggerError.value = ""
  triggerMessage.value = ""
  triggerSql.value = ""
  try {
    const result = await installSourceTriggers({
      source: buildPayload().source,
      tables: qualified,
      profile_id: selectedId.value || undefined,
    })
    triggerMessage.value = result.message
    triggerSql.value = result.sql || ""
    if (result.executed !== false) toast.success(result.message)
    else toast.warning(result.message)
    await loadTriggerStatus()
    triggerPreviewTables.value = []
  } catch (e: any) {
    triggerError.value = formatApiError(e, "Failed to create changelog triggers")
  } finally {
    triggerInstalling.value = ""
  }
}

const confirmTriggerPreview = () => installTriggers(triggerPreviewTables.value)

const installTableTrigger = (schema: string, name: string) => {
  const key = qualifiedKey(schema, name)
  if (!isTableSelected(schema, name)) toggleTable(schema, name)
  return requestTriggerPreview([key])
}

const loadSourceCatalog = async () => {
  if (!canBrowseSource.value) return
  if (!sourceLiveCatalogSupported.value) {
    catalogSchemas.value = []
    catalogTables.value = []
    catalogError.value = ""
    return
  }
  catalogLoading.value = true
  catalogError.value = ""
  try {
    const profileId = selectedId.value || undefined
    // Browse every schema, not just the saved selection: when editing a pipeline the
    // payload carries its selected schemas, which the API treats as a filter — the
    // picker would then show only what is already chosen. Selection state lives in the form.
    const result = await browseSourceCatalog({ ...buildPayload().source, schemas: [], profile_id: profileId })
    catalogSchemas.value = result.schemas || []
    catalogTables.value = result.tables || []
    countDeltas.value = {}
    lastCountAt.value = Date.now()
    if (!catalogSchemas.value.length) {
      catalogError.value = result.message || "No schemas found."
    }
    await loadTriggerStatus()
  } catch (e: any) {
    catalogSchemas.value = []
    catalogTables.value = []
    catalogError.value = formatApiError(e, "Failed to load schemas and tables")
  } finally {
    catalogLoading.value = false
  }
}

const loadTargetCatalog = async () => {
  syncAutofilledSecrets()
  if (!canLoadTargetCatalog.value) {
    if (canTestTarget.value) {
      targetCatalogError.value = "Enter the target password or save a draft first so credentials are stored."
    }
    return
  }
  const payload = buildPayload().target
  if (!payload) return
  targetCatalogLoading.value = true
  targetCatalogError.value = ""
  try {
    const profileId = selectedId.value || undefined
    const result = await browseTargetCatalog({ ...payload, profile_id: profileId })
    targetCatalogSchemas.value = result.schemas || []
    targetCatalogTables.value = result.tables || []
    targetSelectedSchemas.value = targetSelectedSchemas.value.filter((name) =>
      targetCatalogSchemas.value.some((schema) => schema.name.toLowerCase() === name.toLowerCase())
    )
    if (!targetSelectedSchemas.value.length && targetCatalogSchemas.value.length) {
      const withTables = targetCatalogSchemas.value.filter((schema) => schema.table_count > 0)
      targetSelectedSchemas.value = (withTables.length ? withTables : targetCatalogSchemas.value).map(
        (schema) => schema.name,
      )
    }
    if (!targetCatalogSchemas.value.length) {
      targetCatalogError.value = result.message || "No schemas found."
    }
  } catch (e: any) {
    targetCatalogSchemas.value = []
    targetCatalogTables.value = []
    targetCatalogError.value = formatApiError(e, "Failed to load target schemas and tables")
  } finally {
    targetCatalogLoading.value = false
  }
}

const applyCountUpdates = (nextTables: CatalogTable[]) => {
  const keyOf = (table: CatalogTable) => `${table.schema}.${table.name}`.toLowerCase()
  const byKey = new Map(nextTables.map((table) => [keyOf(table), table]))
  const deltas: Record<string, number> = {}
  const merged = catalogTables.value.map((table) => {
    const fresh = byKey.get(keyOf(table))
    if (!fresh || fresh.row_count == null) return table
    if (table.row_count != null && Number(table.row_count) !== Number(fresh.row_count)) {
      deltas[qualifiedKey(table.schema, table.name)] = Number(fresh.row_count) - Number(table.row_count)
    }
    return { ...table, row_count: fresh.row_count, primary_key: fresh.primary_key?.length ? fresh.primary_key : table.primary_key }
  })
  for (const table of nextTables) {
    if (!merged.some((item) => keyOf(item) === keyOf(table))) merged.push(table)
  }
  catalogTables.value = merged
  lastCountAt.value = Date.now()
  if (!Object.keys(deltas).length) return
  countDeltas.value = { ...countDeltas.value, ...deltas }
  if (deltaClearTimer) clearTimeout(deltaClearTimer)
  deltaClearTimer = setTimeout(() => {
    countDeltas.value = {}
  }, 5000)
}

const refreshTableCounts = async (silent = false) => {
  if (!canBrowseSource.value || !sourceLiveCatalogSupported.value || catalogLoading.value || countRefreshing.value) return
  countRefreshing.value = true
  try {
    // Counts only for the selected schemas (cheap); keep the full schema list from loadSourceCatalog.
    const result = await browseSourceCatalog({ ...buildPayload().source, profile_id: selectedId.value || undefined })
    applyCountUpdates(result.tables || [])
    const fresh = new Map((result.schemas || []).map((schema) => [schema.name, schema]))
    catalogSchemas.value = catalogSchemas.value.map((schema) => ({ ...schema, ...(fresh.get(schema.name) ?? {}) }))
  } catch (e: any) {
    if (!silent) catalogError.value = formatApiError(e, "Failed to refresh record counts")
  } finally {
    countRefreshing.value = false
  }
}

const resetCatalog = () => {
  catalogSchemas.value = []
  catalogTables.value = []
  catalogError.value = ""
  tableSearch.value = ""
  countDeltas.value = {}
  lastCountAt.value = null
  countRefreshing.value = false
  inspectTable.value = null
  structureDiff.value = null
  ddlTables.value = null
  triggerKeys.value = []
  triggerInstalling.value = ""
  triggerMessage.value = ""
  triggerError.value = ""
  triggerSql.value = ""
  triggerPreviewTables.value = []
  targetCatalogSchemas.value = []
  targetCatalogTables.value = []
  targetCatalogError.value = ""
  targetSelectedSchemas.value = []
  targetTableSearch.value = ""
}

const transformCatalog: Array<{
  value: TransformPreset
  title: string
  description: string
}> = [
  { value: "none", title: "None", description: "Pass events through without an SMT." },
  { value: "mask_pii", title: "PII mask", description: "Partially mask email and phone fields." },
  { value: "normalize_names", title: "Normalize names", description: "Title-case name fields in the event." },
]

const captureModeCatalog: Array<{
  value: CaptureMode
  title: string
  description: string
}> = [
  {
    value: "logical",
    title: "CDC (WAL)",
    description: "Ongoing change capture from the transaction log. PostgreSQL needs wal_level=logical.",
  },
  {
    value: "trigger",
    title: "Trigger",
    description: "Table triggers write a changelog when WAL/binlog CDC is not available. Works across the supported database types.",
  },
  {
    value: "sync_once",
    title: "One-time sync",
    description: "Copy current rows once, then stop. No ongoing CDC after the snapshot finishes.",
  },
  {
    value: "incremental",
    title: "Incremental",
    description: "Poll tables on updated_at / id. No WAL, no triggers. Best for large tables. Hard deletes are not captured.",
  },
]

const captureModeCaption = computed(
  () => captureModeCatalog.find((item) => item.value === form.captureMode)?.title || "CDC (WAL)"
)

const buildTransforms = (): ProfilePayload["flow"]["transforms"] => {
  if (form.transformPreset === "mask_pii") {
    return [{ name: "std_mask_pii", enabled: true, config: { strategy: "partial_mask", fields: "email,phone" } }]
  }
  if (form.transformPreset === "normalize_names") {
    return [{ name: "std_normalize_name", enabled: true, config: { mode: "title_case" } }]
  }
  return []
}

const presetFromProfile = (profile: CdcProfile): TransformPreset => {
  const name = profile.flow.transforms.find((item) => item.enabled)?.name || ""
  if (name.includes("mask")) return "mask_pii"
  if (name.includes("normalize")) return "normalize_names"
  return "none"
}

const buildLakePayload = (): NonNullable<ProfilePayload["lake"]> => ({
  name: textValue(form.lakeName) || `${textValue(form.name) || "pipeline"}-lake`,
  table_format: form.lakeFormat,
  zone: form.lakeZone,
  change_mode: form.lakeChangeMode,
  catalog_type: form.lakeCatalog,
  namespace: textValue(form.lakeNamespace),
  warehouse: textValue(form.lakeWarehouse),
  connection: {
    endpoint: textValue(form.lakeEndpoint),
    bucket: textValue(form.lakeBucket),
    region: textValue(form.lakeRegion) || "us-east-1",
    access_key: textValue(form.lakeAccessKey),
    secret_key: form.lakeSecretKey,
    catalog_uri: textValue(form.lakeCatalogUri),
    catalog_user: textValue(form.lakeCatalogUser),
    catalog_password: form.lakeCatalogPassword,
  },
})

const buildGovernance = (): GovernanceConfig => ({
  domain: textValue(form.domain),
  dataset: textValue(form.dataset) || textValue(form.name),
  sensitivity: form.sensitivity,
  industry_pack: textValue(form.industryPack) || null,
  pii_fields: [],
})

const optionalPositiveInt = (value: string): number | null => {
  const n = Number(value)
  if (!Number.isFinite(n) || n <= 0) return null
  return Math.floor(n)
}

const buildPayload = (): ProfilePayload => {
  // The picked capture mode is authoritative. Watermark columns linger in the form
  // after switching away from Incremental (they are re-hydrated on every save), and
  // used to flip the saved mode back to incremental — e.g. One-time sync came back
  // as Incremental on the next edit.
  const incrementalCapture = isIncrementalCapture()
  const watermarks = buildSourceWatermarkFields()
  return {
  name: textValue(form.name),
  description: textValue(form.description),
  tags: [...form.tags],
  pattern: form.pattern,
  stream_platform: form.pattern === "source_to_stream" ? form.streamPlatform : null,
  transform_strategy: form.transformPreset === "none" ? "none" : "predefined",
  sink_error_policy: form.sinkErrorPolicy,
  autostart: form.autostart,
  source: {
    source_type: form.sourceType,
    connection: {
      host: textValue(form.host),
      port: textValue(form.port),
      database: textValue(form.database),
      username: textValue(form.username),
      password: form.password,
    },
    schema_mode: form.schemaMode,
    table_mode:
      form.captureMode === "trigger" || incrementalCapture ? "specific_tables" : form.tableMode,
    capture_mode: form.captureMode,
    schemas: form.schemaMode === "specific_schemas" ? splitList(form.schemas) : [],
    tables:
      form.tableMode === "specific_tables" ||
      form.captureMode === "trigger" ||
      incrementalCapture
        ? splitList(form.tables)
        : [],
    changelog_retention_hours:
      form.captureMode === "trigger" ? optionalPositiveInt(form.changelogRetentionHours) : null,
    timestamp_column: watermarks.timestamp_column,
    incrementing_column: watermarks.incrementing_column,
    upsert_key_columns: watermarks.upsert_key_columns,
    table_poll: incrementalCapture ? activeTablePoll() : [],
  },
  target:
    form.pattern === "source_to_target"
      ? {
          name: textValue(form.targetName),
          target_type: form.targetType,
          connection: {
            host: textValue(form.targetHost),
            port: textValue(form.targetPort),
            database: textValue(form.targetDatabase),
            username: textValue(form.targetUsername),
            password: form.targetPassword,
          },
        }
      : null,
  lake: form.pattern === "source_to_lake" ? buildLakePayload() : null,
  governance: buildGovernance(),
  flow: {
    transforms: buildTransforms(),
    schedule: {
      delivery_mode: form.deliveryMode,
      interval_seconds: form.deliveryMode === "micro_batch" ? optionalPositiveInt(form.intervalSeconds) : null,
      full_refresh_minutes: optionalPositiveInt(form.fullRefreshMinutes),
    },
    mappings: (() => {
      if (form.pattern !== "source_to_target") return []
      const upsertKeys = watermarks.upsert_key_columns
      const pkStubs = incrementalCapture ? incrementalPkStubMappings(upsertKeys) : []
      const includeMappings =
        form.mappingMode === "custom" ||
        form.mappings.some((mapping) => (mapping.pk_columns ?? []).length > 0) ||
        pkStubs.length > 0
      if (!includeMappings) return []
      const byKey = new Map<string, TableMapping>()
      for (const mapping of form.mappings) {
        if (!filled(mapping.source_table) || !filled(mapping.target_table)) continue
        byKey.set(mappingKey(mapping.source_schema, mapping.source_table), mapping)
      }
      for (const stub of pkStubs) {
        const key = mappingKey(stub.source_schema, stub.source_table)
        const existing = byKey.get(key)
        if (existing) {
          if (!(existing.pk_columns ?? []).length) existing.pk_columns = stub.pk_columns
        } else {
          byKey.set(key, stub)
        }
      }
      return [...byKey.values()].map((mapping) => {
              // Serialize column_links → backend ColumnMapping[]
              const links = mapping.column_links ?? []
              const columns = links
                .filter((lnk) => lnk.sources.length && lnk.target)
                .map((lnk) => ({
                  source: lnk.sources[0] ?? "",
                  sources: lnk.sources.length > 1 ? lnk.sources : [],
                  target: lnk.target,
                  data_type: "",
                  transform: lnk.fn ?? undefined,
                  transform_args: Object.keys(lnk.fn_args).length ? lnk.fn_args : undefined,
                }))
              const pkRaw = (mapping.pk_columns ?? []).join(",")
              const pk_columns = pkRaw
                .split(",")
                .map((c) => c.trim())
                .filter(Boolean)
              return {
                source_schema: mapping.source_schema,
                source_table: mapping.source_table,
                target_schema: mapping.target_schema,
                target_table: mapping.target_table,
                identical: mappingIsSameName(mapping),
                columns,
                ...(pk_columns.length ? { pk_columns } : {}),
              }
            })
    })(),
  },
  }
}

const resetForm = () => {
  selectedId.value = ""
  editing.value = true
  step.value = "source"
  sourceValidation.value = null
  targetValidation.value = null
  passwordStored.value = false
  targetPasswordStored.value = false
  resetCatalog()
  Object.assign(form, {
    name: "",
    description: "",
    tags: [],
    pattern: "source_to_target",
    sourceType: "postgres",
    host: "cdc-postgres-source",
    port: "5432",
    database: "demo",
    username: "demo",
    password: "",
    schemaMode: "specific_schemas",
    tableMode: "all_tables",
    schemas: "",
    tables: "",
    captureMode: "logical",
    sinkErrorPolicy: "fail_fast",
    transformPreset: "none",
    streamPlatform: "redpanda",
    deliveryMode: "realtime",
    intervalSeconds: "",
    fullRefreshMinutes: "",
    changelogRetentionHours: "",
    timestampColumn: "",
    incrementingColumn: "",
    upsertKeyColumns: "",
  tablePoll: [] as TablePollRule[],
    targetName: "",
    targetType: "postgres",
    targetHost: "cdc-postgres-target",
    targetPort: "5432",
    targetDatabase: "demo",
    targetUsername: "demo",
    targetPassword: "",
    mappingMode: "identical",
    mappings: [],
    lakeName: "",
    lakeFormat: "iceberg",
    lakeZone: "bronze",
    lakeChangeMode: "changelog",
    lakeCatalog: "jdbc",
    lakeEndpoint: "http://cdc-lake:9000",
    lakeBucket: "lake",
    lakeRegion: "us-east-1",
    lakeAccessKey: "cdcadmin",
    lakeSecretKey: "",
    lakeWarehouse: "s3://lake/warehouse",
    lakeNamespace: "",
    lakeCatalogUri: "jdbc:postgresql://cdc-postgres-target:5432/demo",
    lakeCatalogUser: "demo",
    lakeCatalogPassword: "",
    domain: "",
    dataset: "",
    sensitivity: "internal",
    industryPack: "",
    eventLandIceberg: false,
    autostart: true,
  })
}

const payloadFromProfile = (profile: CdcProfile): ProfilePayload => ({
  name: profile.name,
  description: profile.description || "",
  tags: profile.tags ?? [],
  pattern: profile.pattern,
  stream_platform: profile.stream_platform ?? null,
  transform_strategy: profile.transform_strategy,
  sink_error_policy: profile.sink_error_policy,
  autostart: profile.autostart !== false,
  source: profile.source,
  target: profile.target ?? null,
  lake: profile.lake ?? null,
  governance: profile.governance,
  flow: profile.flow,
})

const persistAutostart = async (autostart: boolean) => {
  if (!selected.value) return
  try {
    await updateProfile(selected.value.id, { ...payloadFromProfile(selected.value), autostart })
    toast.success(autostart ? "Autostart enabled." : "Manual start after restart.")
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to update autostart")
  }
}

const tagsSaving = ref(false)
const persistTags = async (tags: string[]) => {
  if (!selected.value) return
  tagsSaving.value = true
  try {
    await updateProfile(selected.value.id, { ...payloadFromProfile(selected.value), tags })
    toast.success(tags.length ? "Tags saved." : "Tags cleared.")
    await load()
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to save tags"))
  } finally {
    tagsSaving.value = false
  }
}

const editSelected = () => {
  if (!selected.value) return
  const profile = selected.value
  selectedId.value = profile.id
  editing.value = true
  step.value = "source"
  sourceValidation.value = null
  targetValidation.value = null
  resetCatalog()
  Object.assign(form, {
    name: profile.name,
    description: profile.description || "",
    tags: [...(profile.tags ?? [])],
    pattern: profile.pattern,
    sourceType: profile.source.source_type,
    host: profile.source.connection.host || "",
    port: profile.source.connection.port || "",
    database: profile.source.connection.database || "",
    username: profile.source.connection.username || "",
    password: "",
    schemaMode: profile.source.schema_mode,
    tableMode:
      profile.source.table_mode ||
      (profile.source.tables?.length ? "specific_tables" : "all_tables"),
    schemas: profile.source.schemas.join(", "),
    tables: profile.source.tables.join(", "),
    captureMode: (String(profile.source.capture_mode || "logical").toLowerCase() as CaptureMode) || "logical",
    sinkErrorPolicy: profile.sink_error_policy || "fail_fast",
    autostart: profile.autostart !== false,
    transformPreset: presetFromProfile(profile),
    streamPlatform: profile.stream_platform || "redpanda",
    deliveryMode: profile.flow.schedule.delivery_mode,
    intervalSeconds: profile.flow.schedule.interval_seconds ? String(profile.flow.schedule.interval_seconds) : "",
    fullRefreshMinutes: profile.flow.schedule.full_refresh_minutes
      ? String(profile.flow.schedule.full_refresh_minutes)
      : "",
    changelogRetentionHours: profile.source.changelog_retention_hours
      ? String(profile.source.changelog_retention_hours)
      : "",
    timestampColumn: profile.source.timestamp_column || "",
    incrementingColumn: profile.source.incrementing_column || "",
    upsertKeyColumns: upsertKeysForForm(profile),
    tablePoll: [...(profile.source.table_poll ?? [])],
    targetName: profile.target?.name || "",
    targetType: (profile.target?.target_type as DatabaseType) || "postgres",
    targetHost: profile.target?.connection.host || "",
    targetPort: profile.target?.connection.port || "5432",
    targetDatabase: profile.target?.connection.database || "",
    targetUsername: profile.target?.connection.username || "",
    targetPassword: "",
    mappingMode: profile.flow.mappings?.length ? "custom" : "identical",
    mappings: (profile.flow.mappings || []).map((mapping) => ({
      source_schema: mapping.source_schema || "",
      source_table: mapping.source_table || "",
      target_schema: mapping.target_schema || "",
      target_table: mapping.target_table || "",
      identical: Boolean(mapping.identical),
      columns: (mapping.columns || []).map((column) => ({
        source: column.source || "",
        target: column.target || "",
        data_type: column.data_type || "",
      })),
      pk_columns: mapping.pk_columns ?? [],
    })),
    lakeName: profile.lake?.name || "",
    lakeFormat: profile.lake?.table_format || "iceberg",
    lakeZone: profile.lake?.zone || "bronze",
    lakeChangeMode: profile.lake?.change_mode || "changelog",
    lakeCatalog: profile.lake?.catalog_type || "jdbc",
    lakeEndpoint: profile.lake?.connection.endpoint || "http://cdc-lake:9000",
    lakeBucket: profile.lake?.connection.bucket || "lake",
    lakeRegion: profile.lake?.connection.region || "us-east-1",
    lakeAccessKey: profile.lake?.connection.access_key === "***" ? "" : (profile.lake?.connection.access_key || "cdcadmin"),
    lakeSecretKey: "",
    lakeWarehouse: profile.lake?.warehouse || "s3://lake/warehouse",
    lakeNamespace: profile.lake?.namespace || "",
    lakeCatalogUri: profile.lake?.connection.catalog_uri || "",
    lakeCatalogUser: profile.lake?.connection.catalog_user || "",
    lakeCatalogPassword: "",
    domain: profile.governance?.domain || "",
    dataset: profile.governance?.dataset || "",
    sensitivity: profile.governance?.sensitivity || "internal",
    industryPack: profile.governance?.industry_pack || "",
    eventLandIceberg: profile.pattern === "source_to_lake",
  })
  // Password fields are not returned by the API for security (replaced with "***").
  // Mark them as "stored" so the UI shows the right placeholder and
  // buildPayload sends an empty string, which preserve_secret_fields keeps.
  passwordStored.value = profile.source.connection.password === "***"
  targetPasswordStored.value = profile.target?.connection.password === "***"
  // Auto-reload the source catalog so schemas/tables appear immediately without
  // the user having to click "Load schemas" again.
  // This uses profile_id so the backend fills in the stored credentials.
  if (passwordStored.value) {
    void nextTick(() => loadSourceCatalog())
  }
}

const load = async () => {
  loading.value = true
  error.value = ""
  try {
    profiles.value = await fetchProfiles()
    if (selectedId.value && !profiles.value.some((profile) => profile.id === selectedId.value)) selectedId.value = ""
    if (selectedId.value) void loadSelectedDlq(selectedId.value)
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load CDC pipelines")
  } finally {
    loading.value = false
  }
}

const refreshPipelineStatuses = async () => {
  if (!selectedId.value || editing.value || document.hidden) return
  try {
    const latest = await fetchProfiles()
    profiles.value = latest
    if (!latest.some((profile) => profile.id === selectedId.value)) selectedId.value = ""
  } catch {
    // Keep the last known status during a transient polling failure. Manual
    // refresh still surfaces request errors to the user.
  }
}

const testTargetConnection = async () => {
  if (!canTestTarget.value) return
  syncAutofilledSecrets()
  validating.value = true
  targetValidation.value = null
  try {
    if (form.pattern === "source_to_lake") {
      const payload = buildPayload().lake
      if (!payload) return
      targetValidation.value = await validateLake(payload)
    } else {
      const payload = buildPayload().target
      if (!payload) return
      const profileId = selectedId.value || undefined
      targetValidation.value = await validateTarget({ ...payload, profile_id: profileId })
    }
    if (targetValidation.value.valid) {
      toast.success("Destination connection validated.")
      if (form.pattern === "source_to_target") await loadTargetCatalog()
    }
    else toast.warning(targetValidation.value.message)
  } catch (e: any) {
    targetValidation.value = { valid: false, message: formatApiError(e, "Destination validation failed") }
  } finally {
    validating.value = false
  }
}

const testConnection = async () => {
  syncAutofilledSecrets()
  validating.value = true
  sourceValidation.value = null
  try {
    const profileId = selectedId.value || undefined
    sourceValidation.value = await validateSource({ ...buildPayload().source, profile_id: profileId })
    if (sourceValidation.value.valid) {
      toast.success("Source connection validated.")
      await loadSourceCatalog()
    } else toast.warning(sourceValidation.value.message)
  } catch (e: any) {
    sourceValidation.value = { valid: false, message: formatApiError(e, "Connection validation failed") }
  } finally {
    validating.value = false
  }
}

const saveDraft = () => save(true)

const closeWizard = () => {
  editing.value = false
}

const save = async (stayInWizard = false) => {
  saving.value = true
  error.value = ""
  try {
    // Capture any browser-autofilled passwords that Vue's v-model may have missed.
    syncAutofilledSecrets()

    if (!filled(form.name)) form.name = `Draft · ${form.sourceType}`
    if (dualCaptureWarning.value) {
      toast.warning(dualCaptureWarning.value)
      return
    }
    const payload = buildPayload()
    const result = selectedId.value
      ? await updateProfile(selectedId.value, payload)
      : await createProfile(payload)
    toast.success(
      stayInWizard
        ? "Draft saved. You can finish this later."
        : selectedId.value
          ? "Pipeline updated."
          : "Draft saved."
    )
    selectedId.value = result.profile.id
    const priorUpsert = textValue(form.upsertKeyColumns)
    const priorTimestamp = textValue(form.timestampColumn)
    applyIncrementalSourceToForm(result.profile)
    if (priorUpsert && !upsertKeysForForm(result.profile)) {
      form.upsertKeyColumns = priorUpsert
    }
    if (priorTimestamp && !result.profile.source.timestamp_column) {
      form.timestampColumn = priorTimestamp
    }

    // Mark passwords as stored and clear plaintext from the form so subsequent
    // saves correctly send an empty password (backend restores from the profile).
    if (form.password) {
      passwordStored.value = true
      form.password = ""
    }
    if (form.targetPassword) {
      targetPasswordStored.value = true
      form.targetPassword = ""
    }
    if (form.lakeSecretKey) {
      form.lakeSecretKey = ""
    }

    if (!stayInWizard) editing.value = false
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to save pipeline")
  } finally {
    saving.value = false
  }
}

const toggleEnabled = async () => {
  if (!selected.value) return
  if (!selected.value.enabled && !canStartSelected.value) {
    toast.warning(startBlockedReason(selected.value.pattern) || "Only replica pipelines can Enable in this phase.")
    return
  }
  try {
    await setEnabled(selected.value.id, !selected.value.enabled)
    toast.success(selected.value.enabled ? "Pipeline disabled." : "Pipeline enabled and reconciled.")
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to change pipeline state")
  }
}

type LifecycleAction = "start" | "pause" | "resume" | "restart" | "stop"

/**
 * A lifecycle action is in flight. Start can take several seconds (column lookup on a
 * remote Oracle, connector creation); a second click meanwhile used to send a second
 * Start, which the orchestrator now refuses with 409 — so don't send it at all.
 */
const actionBusy = ref(false)

const execute = async (action: LifecycleAction) => {
  if (!selected.value || actionBusy.value) return
  if ((action === "start" || action === "restart") && !canStartSelected.value) {
    toast.warning(startBlockedReason(selected.value.pattern) || "Only replica pipelines can Start in this phase.")
    return
  }
  actionBusy.value = true
  try {
    await runAction(selected.value.id, action)
    toast.success(`${action[0].toUpperCase()}${action.slice(1)} requested.`)
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Pipeline action failed")
  } finally {
    actionBusy.value = false
  }
}

const resyncOpen = ref(false)
const initialLoadOpen = ref(false)

// ── Split tables into a new pipeline ─────────────────────────────────────────
const splitOpen = ref(false)

/** Show the new (draft) pipeline after a split. */
const openSplitResult = async (id: string) => {
  splitOpen.value = false
  await load()
  selectPipeline(id)
}

// ── Duplicate as a new pipeline ──────────────────────────────────────────────
const duplicateOpen = ref(false)

/** Show the new (draft) copy after a duplicate. */
const openDuplicateResult = async (id: string) => {
  duplicateOpen.value = false
  await load()
  selectPipeline(id)
}

/** Restart the original so it stops capturing the moved tables. */
const restartAfterSplit = async () => {
  splitOpen.value = false
  await execute("restart")
}
const resyncClearTarget = ref(true)
const resyncRowLimit = ref("")
const resyncing = ref(false)

const startVerifyOpen = ref(false)
const startVerifyClearTarget = ref(true)
const startVerifyRowLimit = ref("100")
const startVerifying = ref(false)

const confirmStartVerify = async () => {
  if (!selected.value) return
  if (!canStartSelected.value) {
    toast.warning(startBlockedReason(selected.value.pattern))
    startVerifyOpen.value = false
    return
  }
  const rowLimit = optionalPositiveInt(startVerifyRowLimit.value)
  if (!rowLimit) {
    toast.warning("Enter a row limit (e.g. 100) for test initial sync.")
    return
  }
  startVerifying.value = true
  try {
    const res = await runAction(selected.value.id, "start", {
      clearTarget: startVerifyClearTarget.value,
      rowLimit,
    })
    const history = (res?.profile as { history?: Array<{ event: string; detail: string }> } | undefined)?.history
    const last = history?.at(-1)
    toast.success(last?.event === "profile_test_initial_sync" ? last.detail : `Test initial sync passed (up to ${rowLimit} rows per table). Pipeline stopped.`)
    startVerifyOpen.value = false
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Test initial sync failed")
  } finally {
    startVerifying.value = false
  }
}

const confirmResync = async () => {
  if (!selected.value) return
  if (!canStartSelected.value) {
    toast.warning(startBlockedReason(selected.value.pattern))
    resyncOpen.value = false
    return
  }
  resyncing.value = true
  try {
    const rowLimit = optionalPositiveInt(resyncRowLimit.value)
    await resyncProfile(selected.value.id, resyncClearTarget.value, rowLimit)
    toast.success(
      selected.value.source?.capture_mode === "trigger"
        ? resyncClearTarget.value
          ? "Target cleared and current source tables copied."
          : "Current source tables copied to the target."
        : resyncClearTarget.value
          ? "Target cleared and replay started from the beginning of the source."
          : "Replay started from the beginning of the source.",
    )
    resyncOpen.value = false
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Resync failed")
  } finally {
    resyncing.value = false
  }
}

// ── Processing log (per-pipeline) ──────────────────────────────────────────

/** Mirror the orchestrator's connector slug: lowercase alnum, others → '-'. */
const connectorSlug = (value: string) =>
  value
    .split("")
    .map((c) => (/[a-zA-Z0-9]/.test(c) ? c.toLowerCase() : "-"))
    .join("")
    .replace(/^-+|-+$/g, "")

/**
 * Connector name tokens used to filter this pipeline's logs. The base name
 * (`cdc-<slug>`) is a prefix shared by every connector the pipeline owns
 * (source, sink, join, initial-load), so filtering on it catches them all.
 */
const pipelineConnectors = computed<string[]>(() => {
  const p = selected.value
  if (!p) return []
  const base = (p.connector_name && p.connector_name.trim())
    ? p.connector_name.trim()
    : `cdc-${connectorSlug(p.name)}`
  const sink = (p.sink_connector_name && p.sink_connector_name.trim())
    ? p.sink_connector_name.trim()
    : `${base}-sink`
  return Array.from(new Set([base, sink]))
})

const loadSelectedDlq = async (id: string) => {
  dlqLoading.value = true
  dlqError.value = ""
  try {
    dlqStatus.value = await fetchDlqStatus(id)
  } catch (e: any) {
    dlqStatus.value = null
    dlqError.value = formatApiError(e, "Failed to load DLQ status")
  } finally {
    dlqLoading.value = false
  }
}

// Collapse stale logs and refresh rejected-record state when switching pipelines.
watch(
  () => selected.value?.id,
  (id) => {
    dlqStatus.value = null
    dlqError.value = ""
    if (id) void loadSelectedDlq(id)
  },
  { immediate: true }
)

watch(
  () => form.captureMode,
  (mode) => {
    if (mode !== "trigger") {
      triggerMessage.value = ""
      triggerError.value = ""
    }
    if (mode === "trigger" || mode === "incremental") {
      form.schemaMode = "specific_schemas"
      form.tableMode = "specific_tables"
    }
    if (mode === "trigger") {
      void loadTriggerStatus()
    }
  }
)

watch(
  () => form.sourceType,
  (type, prev) => {
    if (type === prev) return
    const nextPort = dbTypes.find((item) => item.value === type)?.port
    if (nextPort && (!textValue(form.port) || dbTypes.some((item) => item.port === textValue(form.port)))) {
      form.port = nextPort
    }
    sourceValidation.value = null
  }
)

watch(
  () => form.targetType,
  (type, prev) => {
    if (type === prev) return
    const nextPort = dbTypes.find((item) => item.value === type)?.port
    if (nextPort && (!textValue(form.targetPort) || dbTypes.some((item) => item.port === textValue(form.targetPort)))) {
      form.targetPort = nextPort
    }
    targetValidation.value = null
  }
)


const parseQualified = (value: string) => {
  const parts = value.split(".").map((part) => part.trim()).filter(Boolean)
  if (parts.length >= 2) {
    return { schema: parts.slice(0, -1).join("."), table: parts[parts.length - 1] }
  }
  return { schema: splitList(form.schemas)[0] || "", table: value.trim() }
}

const sourceTablesForMapping = computed(() => {
  if (form.tableMode !== "specific_tables") return []
  return splitList(form.tables)
    .map(parseQualified)
    .filter((item) => item.table)
})
const mappingSchemaOptions = computed(() => {
  const names = new Set<string>([
    ...targetSchemaOptions.value,
    ...selectedSchemaNames.value,
    ...form.mappings.flatMap((mapping) => [mapping.source_schema, mapping.target_schema]),
  ])
  return [...names].map((name) => name.trim()).filter(Boolean).sort((a, b) => a.localeCompare(b))
})
const visibleMappings = computed(() => {
  const query = mappingSearch.value.trim().toLowerCase()
  if (!query) return form.mappings
  return form.mappings.filter((mapping) =>
    `${mapping.source_schema}.${mapping.source_table} ${mapping.target_schema}.${mapping.target_table}`
      .toLowerCase()
      .includes(query)
  )
})


const refreshMappingMode = () => {
  form.mappingMode =
    form.mappings.length > 0 && form.mappings.every(mappingIsSameName) ? "identical" : "custom"
}

const syncMappingsFromSource = () => {
  const sources = sourceTablesForMapping.value
  if (!sources.length) {
    form.mappings = []
    form.mappingMode = "identical"
    return
  }
  const previous = new Map(
    form.mappings.map((mapping) => [mappingKey(mapping.source_schema, mapping.source_table), mapping])
  )
  form.mappings = sources.map((source) => {
    const prev = previous.get(mappingKey(source.schema, source.table))
    if (prev) return prev
    return {
      source_schema: source.schema,
      source_table: source.table,
      target_schema: source.schema,
      target_table: source.table,
      identical: true,
      columns: [],
    }
  })
  refreshMappingMode()
}

const applySameNames = () => {
  form.mappings = form.mappings.map((mapping) => ({
    ...mapping,
    target_schema: mapping.source_schema,
    target_table: mapping.source_table,
    identical: true,
  }))
  refreshMappingMode()
}

const applyTargetSchemaToAll = () => {
  const schema = mappingBulkSchema.value.trim()
  if (!schema) return
  form.mappings = form.mappings.map((mapping) => ({ ...mapping, target_schema: schema }))
  refreshMappingMode()
}

const targetTablesForSchema = (schema: string) =>
  targetCatalogTables.value.filter((table) => table.schema.toLowerCase() === schema.trim().toLowerCase())

const targetTableExists = (schema: string, table: string) =>
  targetTablesForSchema(schema).some((item) => item.name.toLowerCase() === table.trim().toLowerCase())

const mappingTargetMissing = (mapping: TableMapping) =>
  filled(mapping.target_table) &&
  !targetTableExists(mapping.target_schema || mapping.source_schema, mapping.target_table)

const missingTargetMappings = computed(() => form.mappings.filter(mappingTargetMissing))

const ddlRef = (mapping: TableMapping) => ({
  source_schema: mapping.source_schema,
  source_table: mapping.source_table,
  target_schema: mapping.target_schema || mapping.source_schema,
  target_table: mapping.target_table || mapping.source_table,
})

const openCreateTables = (mappings: TableMapping[]) => {
  if (!mappings.length || !buildPayload().target) return
  ddlTables.value = mappings.map(ddlRef)
}

const mappingStatus = (mapping: TableMapping) => {
  if (!filled(mapping.target_table)) return { label: "Need name", tone: "badge-warning" }
  const exists = targetTableExists(mapping.target_schema, mapping.target_table)
  if (mappingIsSameName(mapping)) {
    return exists
      ? { label: "Same name · exists", tone: "badge-success" }
      : { label: "Same name · new", tone: "badge-neutral" }
  }
  return exists
    ? { label: "Renamed · exists", tone: "badge-warning" }
    : { label: "Renamed · new", tone: "badge-neutral" }
}


watch(
  () => form.lakeZone,
  (zone) => {
    if (zone === "bronze") form.lakeChangeMode = "changelog"
    else if (form.lakeChangeMode === "changelog") form.lakeChangeMode = "upsert"
  }
)

const wizardBack = () => {
  if (step.value === "transform") step.value = "source"
  else if (step.value === "target") step.value = "transform"
  else if (step.value === "mapping") step.value = "target"
  else if (step.value === "review") step.value = form.pattern === "source_to_target" ? "mapping" : "target"
}

const goToWizardStep = (next: WizardStep) => {
  if (next === "source" || next === "transform") {
    step.value = next
    return
  }
  if (next === "target") {
    step.value = "target"
    return
  }
  if (next === "mapping") {
    if (!canContinueTarget.value) {
      step.value = "target"
      return
    }
    syncMappingsFromSource()
    if (canTestTarget.value && !targetCatalogTables.value.length) void loadTargetCatalog()
    step.value = "mapping"
    return
  }
  if (form.pattern === "source_to_target" && !canContinueTarget.value) {
    step.value = "target"
    return
  }
  if (form.pattern === "source_to_target" && !canContinueMapping.value) {
    step.value = "mapping"
    return
  }
  step.value = "review"
}

const wizardNext = () => {
  if (step.value === "source") {
    if (!canContinueSource.value) return
    step.value = "transform"
    return
  }
  if (step.value === "transform") {
    step.value = "target"
    return
  }
  if (step.value === "target") {
    syncAutofilledSecrets()
    if (!canContinueTarget.value) return
    if (form.pattern === "source_to_target") {
      syncMappingsFromSource()
      if (canTestTarget.value && !targetCatalogTables.value.length) void loadTargetCatalog()
      step.value = "mapping"
      return
    }
    step.value = "review"
    return
  }
  if (step.value === "mapping") {
    if (!canContinueMapping.value) return
    step.value = "review"
  }
}

const destinationSummary = (profile: {
  pattern: string
  target?: { name?: string; target_type?: string } | null
  lake?: { name?: string; table_format?: string; zone?: string } | null
  stream_platform?: string | null
  governance?: { domain?: string; dataset?: string } | null
}) => {
  if (profile.pattern === "source_to_olake") return "Iceberg (OLake)"
  if (profile.pattern === "source_to_lake") {
    const identity = [profile.governance?.domain, profile.governance?.dataset].filter(Boolean).join(".")
    return profile.lake?.name || identity || `${profile.lake?.zone || "bronze"} ${profile.lake?.table_format || "iceberg"}`
  }
  if (profile.pattern === "source_to_target") {
    return profile.target?.name || profile.target?.target_type || "target"
  }
  return profile.stream_platform || "stream"
}

const captureModeLabel = (profile: CdcProfile) => {
  const mode = profile.source.capture_mode || "logical"
  if (mode === "trigger") return "Trigger"
  if (mode === "sync_once") return "One-time sync"
  if (mode === "incremental") return "Incremental"
  return "CDC (WAL)"
}

const databaseLabel = (type?: string | null) =>
  dbTypes.find((option) => option.value === type)?.label || type || "Database"

const connectionCaption = (connection?: Record<string, string> | null) => {
  if (!connection) return ""
  const host = connection.host || ""
  const database = connection.database || connection.path || ""
  return [host, database].filter(Boolean).join("/") || "Configured connection"
}

const viewerTargetCaption = (profile: CdcProfile) => {
  if (profile.pattern === "source_to_target") return connectionCaption(profile.target?.connection)
  if (profile.pattern === "source_to_lake") {
    return profile.lake?.warehouse || profile.lake?.connection?.endpoint || `${profile.lake?.zone || "bronze"} zone`
  }
  if (profile.pattern === "source_to_olake") return "Apache Iceberg tables"
  return `${profile.stream_platform || "Kafka"} event stream`
}

const viewerTransformCaption = (profile: CdcProfile) =>
  `${captureModeLabel(profile)} · ${profile.flow.schedule.delivery_mode.replace("_", " ")}`

const patternLabel = (profile: CdcProfile) => {
  if (profile.pattern === "source_to_olake") return "Analytics-first"
  if (profile.pattern === "source_to_lake") return "Event-first (Iceberg)"
  if (profile.pattern === "source_to_stream") return "Event-first"
  return "Replica"
}

const pipelineRoute = (profile: CdcProfile) => {
  const sourceHost = profile.source.connection.host || profile.source.source_type
  const sourceDb = profile.source.connection.database || ""
  const source = sourceDb ? `${sourceHost}/${sourceDb}` : sourceHost
  if (profile.pattern === "source_to_target") {
    const targetHost = profile.target?.connection.host || profile.target?.target_type || "target"
    const targetDb = profile.target?.connection.database || ""
    const target = targetDb ? `${targetHost}/${targetDb}` : targetHost
    return `${source} → ${target}`
  }
  if (profile.pattern === "source_to_olake") {
    return `${source} → Iceberg (OLake)`
  }
  if (profile.pattern === "source_to_lake") {
    return `${source} → ${destinationSummary(profile)}`
  }
  return `${source} → ${profile.stream_platform || "stream"}`
}

/** Click a header: first click sorts ascending, second flips, third returns to API order. */
const togglePipelineSort = (key: PipelineSortKey) => {
  if (pipelineSortKey.value !== key) {
    pipelineSortKey.value = key
    pipelineSortDir.value = "asc"
  } else if (pipelineSortDir.value === "asc") {
    pipelineSortDir.value = "desc"
  } else {
    pipelineSortKey.value = null
    pipelineSortDir.value = "asc"
  }
}

const pipelineSortValue = (profile: CdcProfile, key: PipelineSortKey) => {
  switch (key) {
    case "name":
      return profile.name || ""
    case "capture":
      return captureModeLabel(profile)
    case "pattern":
      return patternLabel(profile)
  }
}

const pipelineSortIcon = (key: PipelineSortKey) =>
  pipelineSortKey.value !== key ? ArrowUpDown : pipelineSortDir.value === "asc" ? ArrowUp : ArrowDown

const pipelineAriaSort = (key: PipelineSortKey) =>
  pipelineSortKey.value !== key ? "none" : pipelineSortDir.value === "asc" ? "ascending" : "descending"

const filteredProfiles = computed(() => {
  const query = pipelineSearch.value.trim().toLowerCase()
  const status = pipelineStatusFilter.value
  const key = pipelineSortKey.value
  const dir = pipelineSortDir.value === "asc" ? 1 : -1
  const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true })
  const tag = pipelineTagFilter.value
  const matches = profiles.value.filter((profile) => {
    if (status !== "all" && String(profile.status || "").toLowerCase() !== status) return false
    const tags = profile.tags ?? []
    if (tag === UNTAGGED && tags.length) return false
    if (tag !== "all" && tag !== UNTAGGED && !tags.some((t) => t.toLowerCase() === tag.toLowerCase())) return false
    if (!query) return true
    const haystack = [
      profile.name,
      profile.description,
      ...tags,
      profile.status,
      profile.source.source_type,
      profile.target?.target_type,
      captureModeLabel(profile),
      patternLabel(profile),
      pipelineRoute(profile),
      destinationSummary(profile),
    ]
      .filter(Boolean)
      .join(" ")
      .toLowerCase()
    return haystack.includes(query)
  })
  if (!key) return matches
  // Stable: equal keys keep the API order, and ties on capture/pattern fall back to name.
  return [...matches].sort((a, b) => {
    const primary = collator.compare(pipelineSortValue(a, key), pipelineSortValue(b, key))
    if (primary) return primary * dir
    return key === "name" ? 0 : collator.compare(a.name || "", b.name || "")
  })
})

/** Every tag in use, first spelling wins, sorted for the filter and suggestions. */
const allTags = computed(() => {
  const seen = new Map<string, string>()
  for (const profile of profiles.value) {
    for (const tag of profile.tags ?? []) {
      const fold = tag.toLowerCase()
      if (!seen.has(fold)) seen.set(fold, tag)
    }
  }
  return [...seen.values()].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base", numeric: true }))
})

/** Tag view: one group per tag (a pipeline with two tags shows in both), untagged last. */
const profilesByTag = computed(() => {
  const groups = new Map<string, { key: string; label: string; items: CdcProfile[] }>()
  for (const tag of allTags.value) groups.set(tag.toLowerCase(), { key: tag, label: tag, items: [] })
  const untagged: CdcProfile[] = []
  for (const profile of filteredProfiles.value) {
    const tags = profile.tags ?? []
    if (!tags.length) untagged.push(profile)
    for (const tag of tags) groups.get(tag.toLowerCase())?.items.push(profile)
  }
  const out = [...groups.values()].filter((g) => g.items.length)
  if (untagged.length) out.push({ key: UNTAGGED, label: "Untagged", items: untagged })
  return out
})

/**
 * Capture view: one group per capture mode, in catalog order, empty groups hidden.
 * Missing/unknown modes fold into "logical" so headers agree with captureModeLabel().
 */
const profilesByCapture = computed(() => {
  const groups = new Map(
    captureModeCatalog.map((c) => [c.value, { key: c.value, label: c.title, description: c.description, items: [] as CdcProfile[] }])
  )
  for (const profile of filteredProfiles.value) {
    const mode = String(profile.source.capture_mode || "logical").toLowerCase() as CaptureMode
    ;(groups.get(mode) ?? groups.get("logical")!).items.push(profile)
  }
  return [...groups.values()].filter((g) => g.items.length)
})

const visibleProfileIds = computed(() => new Set(filteredProfiles.value.map((p) => p.id)))

const statusFilterOptions = computed(() => {
  const seen = new Set<string>()
  for (const profile of profiles.value) {
    const status = String(profile.status || "").trim().toLowerCase()
    if (status) seen.add(status)
  }
  return ["all", ...[...seen].sort()]
})

const openEdit = (profile: CdcProfile) => {
  selectedId.value = profile.id
  editSelected()
}

const selectPipeline = (id: string) => {
  selectedId.value = id
  editing.value = false
}

// Sidebar "CDC Pipelines" while already here: back to the list, like a fresh visit.
const { onNavReselect } = useNavReselect()
onNavReselect("/pipelines", () => {
  editing.value = false
  selectedId.value = ""
})

const confirmDelete = async () => {
  const profile = pendingDelete.value
  if (!profile) return
  deleting.value = true
  error.value = ""
  try {
    await deleteProfile(profile.id)
    toast.success(`Deleted ${profile.name}.`)
    if (selectedId.value === profile.id) selectedId.value = ""
    pendingDelete.value = null
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to delete pipeline")
  } finally {
    deleting.value = false
  }
}

const sourceCaption = computed(() =>
  canContinueSource.value ? `${form.host}/${form.database}` : "",
)
const transformCaption = computed(() =>
  transformCatalog.find((item) => item.value === form.transformPreset)?.title || "None",
)
const targetCaption = computed(() => {
  if (form.pattern === "source_to_olake") return "Iceberg (OLake)"
  if (form.pattern === "source_to_stream") return form.streamPlatform
  if (form.pattern === "source_to_lake") {
    return form.lakeName || `${form.lakeZone} ${form.lakeFormat}`
  }
  return filled(form.targetName) ? form.targetName : `${form.targetHost}/${form.targetDatabase}`
})
const captureCaption = computed(() => {
  const schema = form.schemaMode === "all_schemas" ? "all schemas" : `schemas ${textValue(form.schemas) || "—"}`
  const table = form.tableMode === "all_tables" ? "all tables" : `tables ${textValue(form.tables) || "—"}`
  return `${captureModeCaption.value} · ${schema} · ${table}`
})
const mappingCaption = computed(() => {
  if (form.pattern === "source_to_olake") return "OLake stack (Phase 3)"
  if (form.pattern === "source_to_lake") return `${form.lakeZone} · ${form.lakeChangeMode}`
  if (form.pattern !== "source_to_target") return "Not used for Event-first"
  if (!sourceTablesForMapping.value.length || form.mappingMode === "identical") {
    return "Same names as source"
  }
  const renamed = form.mappings.filter((mapping) => !mappingIsSameName(mapping)).length
  return `${form.mappings.length} tables · ${renamed} renamed`
})

watch(
  () => step.value,
  (next) => {
    if (next === "mapping") {
      syncMappingsFromSource()
      if (canTestTarget.value && !targetCatalogTables.value.length) void loadTargetCatalog()
    }
  }
)

watch(
  () =>
    editing.value &&
    step.value === "source" &&
    form.schemaMode === "specific_schemas" &&
    selectedSchemaNames.value.length > 0 &&
    catalogTables.value.length > 0 &&
    canBrowseSource.value,
  (active) => {
    if (countPollTimer) {
      clearInterval(countPollTimer)
      countPollTimer = null
    }
    if (!active) return
    // Load record counts once when schemas are selected (no auto-refresh polling).
    void refreshTableCounts(true)
  },
  { immediate: true }
)

const selectedFlow = computed(() => {
  const profile = selected.value
  if (!profile) return null
  return {
    sourceLabel: databaseLabel(profile.source.source_type),
    sourceCaption: connectionCaption(profile.source.connection),
    sourceImage: dbIconPath(profile.source.source_type),
    transformCaption: viewerTransformCaption(profile),
    targetLabel: destinationSummary(profile),
    targetCaption: viewerTargetCaption(profile),
    targetImage: profile.pattern === "source_to_target" ? dbIconPath(profile.target?.target_type) : "",
  }
})

const onDetailAction = (action: PipelineDetailAction) => {
  switch (action) {
    case "edit":
      editSelected()
      return
    case "toggle-enabled":
      void toggleEnabled()
      return
    case "delete":
      pendingDelete.value = selected.value
      return
    case "test-sync":
      startVerifyOpen.value = true
      return
    case "resync":
      resyncOpen.value = true
      return
    case "initial-load":
      initialLoadOpen.value = true
      return
    case "split":
      splitOpen.value = true
      return
    case "duplicate":
      duplicateOpen.value = true
      return
    default:
      void execute(action)
  }
}

onMounted(() => {
  void load()
  statusPollTimer = setInterval(() => void refreshPipelineStatuses(), 5000)
})
onUnmounted(() => {
  if (countPollTimer) clearInterval(countPollTimer)
  if (deltaClearTimer) clearTimeout(deltaClearTimer)
  if (statusPollTimer) clearInterval(statusPollTimer)
})
</script>

<template>
  <div class="page-container">
    <AppPageHeader
      title="CDC Pipelines"
      description="Database → database first. Capture from a source, write to a target, no Connect JSON."
    >
      <AppButton :disabled="loading" @click="load">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton variant="primary" @click="resetForm">
        <Plus class="h-4 w-4" :stroke-width="1.75" />
        New Pipeline
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <section v-if="editing" class="stat-card">
        <form class="contents" @submit.prevent>
        <PipelineFlow
          variant="designer"
          :pattern="form.pattern"
          :active-node="step"
          :source-configured="canContinueSource"
          :transform-configured="form.transformPreset !== 'none'"
          :target-configured="canContinueTarget"
          :mapping-configured="canContinueMapping && form.mappings.length > 0"
          :review-ready="step === 'review'"
          :source-caption="sourceCaption"
          :transform-caption="transformCaption"
          :target-caption="targetCaption"
          :mapping-caption="mappingCaption"
          @select="(id) => goToWizardStep(id)"
        />
        <div class="mb-4 flex items-center justify-center gap-1.5 text-sm font-medium text-foreground">
          <span>Pipeline steps</span>
          <AppInfoHint
            text="Click a step above to jump back. Connect the target first, then map source tables one by one. Save a draft at any time and continue later."
          />
        </div>
        <div class="mb-6 flex flex-wrap items-end justify-between gap-3 rounded-xl border border-border bg-muted/40 p-3">
          <label class="min-w-[220px] flex-1">
            <AppFormLabel
              label="Pipeline name"
              hint="Optional — saved as Draft with the source database type when empty."
            />
            <AppInput v-model="form.name" :placeholder="`Optional — saved as Draft · ${form.sourceType} if empty`" />
          </label>
          <div class="flex flex-wrap gap-2">
            <AppButton :disabled="saving" @click="closeWizard">Close</AppButton>
            <AppButton variant="primary" :disabled="saving" @click="saveDraft">
              <Save class="h-4 w-4" :stroke-width="1.75" />
              {{ saving ? "Saving..." : "Save draft" }}
            </AppButton>
          </div>
        </div>

        <div v-if="step === 'source'" class="space-y-5">
          <div>
            <AppSectionTitle title="Source" hint="Where should Hyper CDC capture changes from? Choose all schemas/tables or a subset." />
          </div>
          <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
            <div class="md:col-span-2">
              <span class="auth-label">Database Type</span>
              <div class="grid grid-cols-4 gap-2 sm:grid-cols-4 lg:grid-cols-8">
                <button
                  v-for="t in dbTypes"
                  :key="t.value"
                  type="button"
                  class="flex flex-col items-center gap-1.5 rounded-xl border px-2 py-2.5 text-center transition-all hover:border-brand-500/60 hover:bg-brand-500/5"
                  :class="form.sourceType === t.value
                    ? 'border-brand-500 bg-brand-500/10 text-foreground shadow-sm'
                    : 'border-border bg-card text-muted-foreground'"
                  @click="form.sourceType = t.value as DatabaseType; form.port = t.port"
                >
                  <img :src="t.icon" :alt="t.label" class="h-7 w-7 object-contain" />
                  <span class="text-xs font-medium leading-tight">{{ t.label }}</span>
                </button>
              </div>
            </div>
            <label>
              <span class="auth-label">Host</span>
              <AppInput v-model="form.host" placeholder="db.internal.example" />
            </label>
            <label>
              <span class="auth-label">Port</span>
              <AppInput v-model="form.port" inputmode="numeric" placeholder="5432" />
            </label>
            <label>
              <span class="auth-label">Database</span>
              <AppInput v-model="form.database" placeholder="orders" />
            </label>
            <div class="grid grid-cols-2 gap-3">
              <label>
                <span class="auth-label">Username</span>
                <AppInput v-model="form.username" placeholder="cdc_reader" />
              </label>
              <label>
                <span class="auth-label">Password</span>
                <AppInput
                  v-model="form.password"
                  type="password"
                  :placeholder="passwordStored && !form.password ? '•••••• (stored)' : 'Database password'"
                  @input="passwordStored = false"
                />
                <p v-if="passwordStored && !form.password" class="mt-1 text-xs text-surface-400">
                  Password is saved. Leave blank to keep existing, or type to change it.
                </p>
              </label>
            </div>
            <div class="md:col-span-2 rounded-xl border border-border bg-muted/40 p-4">
              <template v-if="form.captureMode === 'trigger'">
                <div class="flex items-start gap-3">
                  <TriangleAlert class="mt-0.5 h-5 w-5 shrink-0 text-yellow-500" :stroke-width="1.75" />
                  <div>
                    <div class="flex items-center gap-1">
                      <p class="text-sm font-medium text-foreground">Trigger sink uses a dead-letter queue</p>
                      <AppInfoHint
                        text="This preserves existing trigger behavior. Rejected target rows continue to the DLQ with error-context headers and must be corrected and replayed."
                      />
                    </div>
                  </div>
                </div>
              </template>
              <label v-else>
                <AppFormLabel
                  label="Rejected record policy"
                  hint="Fail fast prevents silent target gaps. DLQ keeps the sink running, but rejected rows remain missing until correction and replay."
                />
                <AppSelect v-model="form.sinkErrorPolicy" aria-label="Rejected record policy">
                  <option value="fail_fast">Fail fast — stop the sink</option>
                  <option value="dlq">Dead-letter queue — continue processing</option>
                </AppSelect>
              </label>
            </div>
          </div>
          <div class="space-y-3">
            <span class="auth-label">Capture mode</span>
            <div class="grid grid-cols-1 gap-3 md:grid-cols-2">
              <button
                v-for="item in captureModeCatalog"
                :key="item.value"
                type="button"
                class="list-option"
                :class="{
                  'list-option-active': form.captureMode === item.value,
                }"
                @click="form.captureMode = item.value"
              >
                <div class="flex items-center gap-1">
                  <p class="text-sm font-medium">{{ item.title }}</p>
                  <AppInfoHint :text="item.description" />
                </div>
              </button>
            </div>
            <SourceCdcReadiness
              :source-type="form.sourceType"
              :capture-mode="form.captureMode"
              :username="form.username"
              :database="form.database"
              :tables="form.tableMode === 'specific_tables' ? form.tables : ''"
            />
            <div v-if="form.captureMode === 'trigger'" class="grid grid-cols-1 gap-4 md:grid-cols-2">
              <div class="md:col-span-2 grid grid-cols-1 gap-4 sm:grid-cols-2 sm:items-end">
                <label>
                  <AppFormLabel
                    label="Poll cadence"
                    hint="Realtime polls the changelog every 1s. Interval reduces load on production sources."
                  />
                  <AppSelect v-model="form.deliveryMode" aria-label="Trigger poll cadence">
                    <option value="realtime">Realtime (every 1s)</option>
                    <option value="micro_batch">Interval (protect production)</option>
                  </AppSelect>
                </label>
                <label :class="{ 'opacity-60': form.deliveryMode !== 'micro_batch' }">
                  <AppFormLabel
                    label="Interval (seconds)"
                    hint="JDBC polls only the changelog table, not your live tables. 300 = every 5 minutes."
                  />
                  <AppInput
                    v-model="form.intervalSeconds"
                    inputmode="numeric"
                    placeholder="300"
                    :disabled="form.deliveryMode !== 'micro_batch'"
                  />
                </label>
              </div>
              <label>
                <AppFormLabel
                  label="Changelog retention (hours)"
                  hint="Deletes changelog rows older than this. Must be larger than expected lag. Leave blank to disable. Oracle prune is SQL for a DBA job."
                />
                <AppInput v-model="form.changelogRetentionHours" inputmode="numeric" placeholder="Leave blank to keep all rows" />
              </label>
            </div>
            <div v-else-if="form.captureMode === 'incremental'" class="grid grid-cols-1 gap-4 md:grid-cols-2">
              <label>
                <AppFormLabel
                  label="Timestamp column"
                  hint="Required for text/composite PK upsert. Optional for append-only numeric-id tables. Same name on every selected table. NOT NULL and indexed."
                />
                <AppInput v-model="form.timestampColumn" placeholder="UPDATE_DATE" />
              </label>
              <label class="md:col-span-2">
                <AppFormLabel
                  label="Upsert key columns"
                  hint="Target PK for upsert (text/composite OK). Comma-separated — e.g. BRANCH_NO, PRO_CODE. With timestamp above, leave Incrementing empty; JDBC uses timestamp poll mode."
                />
                <AppInput v-model="form.upsertKeyColumns" placeholder="BRANCH_NO, PRO_CODE" />
              </label>
              <label>
                <AppFormLabel
                  label="Incrementing column (numeric)"
                  hint="Optional when upsert keys + timestamp are set. Single numeric monotonic column for JDBC poll (e.g. id). Not for varchar PKs."
                />
                <AppInput v-model="form.incrementingColumn" placeholder="id" />
              </label>
              <TablePollEditor
                v-model="form.tablePoll"
                class="md:col-span-2"
                :tables="splitList(form.tables)"
              />
              <div class="md:col-span-2 grid grid-cols-1 gap-4 sm:grid-cols-2 sm:items-end">
                <label>
                  <AppFormLabel
                    label="Poll cadence"
                    hint="Realtime polls watermarks every 1s. Interval reduces JDBC load on large production tables."
                  />
                  <AppSelect v-model="form.deliveryMode" aria-label="Incremental poll cadence">
                    <option value="realtime">Realtime (every 1s)</option>
                    <option value="micro_batch">Interval (protect production)</option>
                  </AppSelect>
                </label>
                <label :class="{ 'opacity-60': form.deliveryMode !== 'micro_batch' }">
                  <AppFormLabel
                    label="Interval (seconds)"
                    hint="JDBC reads only rows newer than the last watermark. 300 = every 5 minutes. Prefer this on large production tables."
                  />
                  <AppInput
                    v-model="form.intervalSeconds"
                    inputmode="numeric"
                    placeholder="300"
                    :disabled="form.deliveryMode !== 'micro_batch'"
                  />
                </label>
              </div>
              <label class="md:col-span-2">
                <AppFormLabel
                  label="Full refresh (minutes)"
                  hint="Reloads the whole table on a cycle so updates and deletes reach the target. Leave blank to disable. Costs a full table read each time — keep it well above the poll interval."
                />
                <AppInput v-model="form.fullRefreshMinutes" inputmode="numeric" placeholder="off" />
              </label>
            </div>
          </div>
          <div class="space-y-4">
            <div class="flex flex-wrap items-end justify-between gap-3">
              <label class="min-w-[220px] flex-1">
                <span class="auth-label">Schemas</span>
                <AppSelect v-model="form.schemaMode" aria-label="Schema selection mode">
                  <option value="all_schemas">All schemas</option>
                  <option value="specific_schemas">Selected schemas</option>
                </AppSelect>
              </label>
              <AppButton :disabled="catalogLoading || !canBrowseSource || !sourceLiveCatalogSupported" @click="loadSourceCatalog">
                <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': catalogLoading }" :stroke-width="1.75" />
                {{ catalogLoading ? "Loading..." : "Load schemas" }}
              </AppButton>
            </div>
            <AppContextHint
              v-if="!sourceLiveCatalogSupported"
              tone="yellow"
              label="Manual schema entry"
              :hint="`Live catalog browsing is not available for ${form.sourceType}. Enter schema and table names below. DB2 and Informix require manual entry.`"
            />
            <AppContextHint
              v-else-if="form.schemaMode === 'all_schemas'"
              tone="neutral"
              label="All schemas"
              hint="Every schema and table in this database will be captured. Switch to Selected schemas to pick a subset."
            />
            <div v-else class="space-y-4">
              <AppContextHint tone="neutral" label="Schemas & tables" :hint="schemaPickerHint" />
              <p v-if="catalogError" class="text-sm text-red-700 dark:text-red-400">{{ catalogError }}</p>
              <AppContextHint
                v-else-if="!catalogSchemas.length && !catalogLoading"
                tone="neutral"
                label="Load schemas"
                hint="Test the connection or click Load schemas, then pick schemas to capture. Nothing is selected until you check a schema."
              />

              <!-- Saved selection summary — visible while catalog is loading or hasn't loaded yet -->
              <div
                v-if="catalogLoading || (!catalogSchemas.length && (selectedSchemaNames.length || selectedTableKeys.length))"
                class="rounded-xl border border-brand-500/30 bg-brand-500/5 p-3 space-y-2"
              >
                <div class="flex items-center gap-2">
                  <span v-if="catalogLoading" class="h-3.5 w-3.5 animate-spin rounded-full border-2 border-brand-500/40 border-t-brand-500" />
                  <p class="text-xs font-semibold text-brand-400">
                    {{ catalogLoading ? "Reloading catalog…" : "Saved selection" }}
                  </p>
                </div>
                <div v-if="selectedSchemaNames.length" class="flex flex-wrap gap-1.5">
                  <span class="text-xs text-surface-400">Schemas:</span>
                  <span
                    v-for="schema in selectedSchemaNames"
                    :key="schema"
                    class="badge-neutral !text-[10px]"
                  >{{ schema }}</span>
                </div>
                <div v-if="selectedTableKeys.length" class="flex flex-wrap gap-1.5">
                  <span class="text-xs text-surface-400">Tables:</span>
                  <span
                    v-for="table in selectedTableKeys"
                    :key="table"
                    class="badge-neutral !text-[10px]"
                  >{{ table }}</span>
                </div>
                <p v-if="!catalogLoading" class="text-xs text-surface-500">Click <strong>Load schemas</strong> to edit your selection.</p>
              </div>
              <div v-if="schemaOptions.length" class="grid grid-cols-1 gap-2 md:grid-cols-2">
                <label
                  v-for="schema in schemaOptions"
                  :key="schema"
                  class="list-option flex cursor-pointer items-center gap-3"
                  :class="{ 'list-option-active': isSchemaSelected(schema) }"
                >
                  <input
                    type="checkbox"
                    class="h-4 w-4 accent-brand-500"
                    :checked="isSchemaSelected(schema)"
                    @change="toggleSchema(schema)"
                  />
                  <span class="min-w-0">
                    <span class="block text-sm font-medium">{{ schema }}</span>
                    <span class="mt-0.5 block text-xs text-muted-foreground">{{ tableCountFor(schema) }} tables</span>
                  </span>
                </label>
              </div>
              <div v-if="selectedSchemaNames.length" class="space-y-3 rounded-xl border border-border bg-muted/40 p-4">
                <div class="flex flex-wrap items-center justify-between gap-3">
                  <p class="text-sm font-medium">Tables in {{ selectedSchemaNames.join(", ") }}</p>
                  <div class="flex flex-wrap gap-2">
                    <AppButton
                      v-if="form.captureMode === 'trigger'"
                      :disabled="Boolean(triggerInstalling) || !canBrowseSource || !selectedTableKeys.length"
                      @click="requestTriggerPreview(selectedTableKeys)"
                    >
                      <Zap class="h-4 w-4" :stroke-width="1.75" />
                      {{ triggerInstalling === "*" ? "Creating..." : "Create triggers on selected" }}
                    </AppButton>
                    <AppButton :disabled="!tablesForSelectedSchemas.length" @click="selectAllVisibleTables">Select all</AppButton>
                    <AppButton :disabled="!selectedTableKeys.length" @click="clearVisibleTables">Clear</AppButton>
                  </div>
                </div>
                <AppInput v-model="tableSearch" placeholder="Filter tables..." />
                <p v-if="!catalogTables.length && !catalogLoading" class="text-sm text-muted-foreground">
                  Load schemas to show tables for the selected schema.
                </p>
                <p v-else-if="!tablesForSelectedSchemas.length" class="text-sm text-muted-foreground">
                  No tables match this filter.
                </p>
                <div v-else class="max-h-96 space-y-3 overflow-auto pr-1">
                  <div v-for="group in tablesGrouped" :key="group.schema" class="space-y-2">
                    <p class="text-xs font-medium text-muted-foreground">{{ group.schema }}</p>
                    <div
                      v-for="table in group.tables"
                      :key="`${table.schema}.${table.name}`"
                      class="flex items-center justify-between gap-3 rounded-xl border border-border px-3 py-2"
                      :class="{
                        'border-brand-500/60 bg-brand-500/10': isTableSelected(table.schema, table.name) && !countDeltaFor(table.schema, table.name),
                        'border-emerald-500/50 bg-emerald-500/10': countDeltaFor(table.schema, table.name) > 0,
                        'border-red-500/50 bg-red-500/10': countDeltaFor(table.schema, table.name) < 0,
                      }"
                    >
                      <label class="flex min-w-0 flex-1 cursor-pointer items-center gap-3">
                        <input
                          type="checkbox"
                          class="h-4 w-4 accent-brand-500"
                          :checked="isTableSelected(table.schema, table.name)"
                          @change="toggleTable(table.schema, table.name)"
                        />
                        <span class="min-w-0">
                          <span class="block truncate font-mono text-sm">{{ table.name }}</span>
                          <span
                            class="mt-0.5 inline-flex items-center gap-1 text-xs"
                            :class="table.primary_key?.length ? 'text-muted-foreground' : 'text-yellow-700 dark:text-yellow-400'"
                          >
                            <KeyRound class="h-3 w-3" :stroke-width="1.75" />
                            {{ formatPrimaryKey(table.primary_key) }}
                          </span>
                          <span
                            v-if="form.captureMode === 'trigger' && isTriggerInstalled(table.schema, table.name)"
                            class="mt-0.5 block text-xs text-emerald-700 dark:text-emerald-400"
                          >
                            Trigger ready
                          </span>
                        </span>
                      </label>
                      <span class="flex shrink-0 items-center gap-2">
                        <span class="text-right text-xs text-muted-foreground">
                          <span>{{ formatRecordCount(table.row_count) }}</span>
                          <span
                            v-if="countDeltaFor(table.schema, table.name)"
                            class="ml-1 font-medium"
                            :class="countDeltaFor(table.schema, table.name) > 0 ? 'text-emerald-600 dark:text-emerald-400' : 'text-red-600 dark:text-red-400'"
                          >
                            {{ countDeltaFor(table.schema, table.name) > 0 ? "+" : "" }}{{ countDeltaFor(table.schema, table.name).toLocaleString() }}
                          </span>
                        </span>
                        <AppButton
                          v-if="form.captureMode === 'trigger'"
                          size="sm"
                          :disabled="Boolean(triggerInstalling) || !canBrowseSource"
                          @click="installTableTrigger(table.schema, table.name)"
                        >
                          <Zap class="h-4 w-4" :stroke-width="1.75" />
                          {{ triggerInstalling === `${table.schema}.${table.name}` ? "Creating..." : isTriggerInstalled(table.schema, table.name) ? "Recreate" : "Create trigger" }}
                        </AppButton>
                        <AppButton size="sm" @click="inspectTable = { side: 'source', schema: table.schema, name: table.name }">
                          <Eye class="h-4 w-4" :stroke-width="1.75" />
                          View
                        </AppButton>
                      </span>
                    </div>
                  </div>
                </div>
                <div class="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
                  <span>{{ selectedTableKeys.length ? `${selectedTableKeys.length} selected` : "Table selection" }}</span>
                  <AppInfoHint :text="tableSelectionHint + (lastCountLabel ? ` ${lastCountLabel}` : '')" />
                </div>
                <p v-if="triggerError" class="text-sm text-red-700 dark:text-red-400">{{ triggerError }}</p>
                <p v-else-if="triggerMessage" class="text-sm text-emerald-700 dark:text-emerald-400">{{ triggerMessage }}</p>
                <pre v-if="triggerSql" class="code-block max-h-48 overflow-auto whitespace-pre-wrap text-xs">{{ triggerSql }}</pre>
              </div>
            </div>
          </div>
          <div class="flex flex-wrap items-center justify-between gap-3 border-t border-border pt-5">
            <div>
              <AppButton :disabled="validating || !canBrowseSource" @click="testConnection">
                <CheckCircle2 class="h-4 w-4" :stroke-width="1.75" />
                {{ validating ? "Testing..." : "Test Connection" }}
              </AppButton>
              <p
                v-if="sourceValidation"
                class="mt-2 text-sm"
                :class="sourceValidation.valid ? 'text-emerald-700 dark:text-emerald-400' : 'text-red-700 dark:text-red-400'"
              >
                {{ sourceValidation.message }}
              </p>
            </div>
            <AppContextHint
              v-if="incrementalSourceHint"
              tone="yellow"
              label="Incremental setup"
              :hint="incrementalSourceHint"
            />
            <AppButton variant="primary" :disabled="!canContinueSource" @click="wizardNext">
              Next: Transform <ArrowRight class="h-4 w-4" :stroke-width="1.75" />
            </AppButton>
          </div>
        </div>

        <div v-else-if="step === 'transform'" class="space-y-5">
          <div>
            <AppSectionTitle title="Transform" hint="Optional single-message transforms, the same SMT step as Debezium Stage. Skip with None." />
          </div>
          <div class="grid grid-cols-1 gap-3 md:grid-cols-3">
            <button
              v-for="item in transformCatalog"
              :key="item.value"
              type="button"
              class="list-option"
              :class="{ 'list-option-active': form.transformPreset === item.value }"
              @click="form.transformPreset = item.value"
            >
              <div class="flex items-start gap-3">
                <div class="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-white">
                  <img
                    v-if="item.value !== 'none'"
                    src="/images/cdc/pipeline-transform.png"
                    alt=""
                    class="h-8 w-8 object-contain"
                  />
                  <Filter v-else class="h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
                </div>
                <div class="min-w-0 text-left">
                  <div class="flex items-center gap-1">
                    <p class="text-sm font-medium">{{ item.title }}</p>
                    <AppInfoHint :text="item.description" />
                  </div>
                </div>
              </div>
            </button>
          </div>
          <div class="flex justify-between border-t border-border pt-5">
            <AppButton @click="wizardBack">Back</AppButton>
            <AppButton variant="primary" @click="wizardNext">
              Next: Target <ArrowRight class="h-4 w-4" :stroke-width="1.75" />
            </AppButton>
          </div>
        </div>

        <div v-else-if="step === 'target'" class="space-y-5">
          <div>
            <AppSectionTitle title="Destination" hint="Replica is the current product. Event-first and Analytics-first can be saved as drafts." />
          </div>
          <div class="grid grid-cols-1 gap-3 md:grid-cols-3">
            <button
              type="button"
              class="list-option"
              :class="{ 'list-option-active': form.pattern === 'source_to_target' }"
              @click="selectReplicaDestination"
            >
              <div class="flex items-center gap-1">
                <p class="text-sm font-medium">Database replica</p>
                <AppInfoHint text="Source → target database. Upsert and delete, auto-create tables. This is the current product path." />
              </div>
            </button>
            <button
              type="button"
              class="list-option"
              :class="{ 'list-option-active': isEventFirstPattern(form.pattern) }"
              @click="selectEventDestination"
            >
              <div class="flex items-center gap-1">
                <p class="text-sm font-medium">Event-first <span class="text-xs font-normal text-muted-foreground">Phase 2</span></p>
                <AppInfoHint text="Debezium publishes change events to Kafka. Save a draft now; Start comes in Phase 2." />
              </div>
            </button>
            <button
              type="button"
              class="list-option"
              :class="{ 'list-option-active': form.pattern === 'source_to_olake' }"
              @click="selectAnalyticsDestination"
            >
              <div class="flex items-center gap-1">
                <p class="text-sm font-medium">Analytics-first <span class="text-xs font-normal text-muted-foreground">Phase 3</span></p>
                <AppInfoHint text="OLake reads the source into Iceberg. Separate stack — never with Debezium on the same database." />
              </div>
            </button>
          </div>
          <label v-if="isEventFirstPattern(form.pattern)" class="flex items-start gap-3 rounded-xl border border-border bg-muted/40 p-4">
            <input
              type="checkbox"
              class="mt-1 h-4 w-4 rounded border-border"
              :checked="form.eventLandIceberg"
              @change="setEventLandIceberg(($event.target as HTMLInputElement).checked)"
            >
            <span>
              <span class="inline-flex items-center gap-1 text-sm font-medium">
                Also land Iceberg from Kafka
                <AppInfoHint text="Optional Event-first add-on (source_to_lake). Not OLake. Needs the lake Connect image in Phase 2b." />
              </span>
            </span>
          </label>
          <AppContextHint
            v-if="dualCaptureWarning"
            tone="red"
            label="Source conflict"
            :hint="dualCaptureWarning"
          />
          <AppContextHint
            v-if="form.pattern === 'source_to_target' && form.captureMode === 'logical'"
            tone="emerald"
            label="Logical replica"
            hint="Uses the capture mode from Source. PostgreSQL needs wal_level=logical. Leave Transform as None for a straight copy."
          />
          <AppContextHint
            v-else-if="form.pattern === 'source_to_target' && form.captureMode === 'trigger'"
            tone="yellow"
            label="Trigger replica"
            hint="JDBC source on the changelog table, not Debezium. Create triggers on Source before Start. The JDBC sink upserts into the target."
          />
          <AppContextHint
            v-else-if="form.pattern === 'source_to_target' && form.captureMode === 'incremental'"
            tone="blue"
            label="Incremental replica"
            hint="JDBC poll on live tables (not Debezium, not triggers). Sink upserts on incrementing or upsert key columns (text/composite OK with a timestamp). Hard deletes on the source are not applied on the target."
          />
          <AppContextHint
            v-else-if="form.pattern === 'source_to_target' && form.captureMode === 'sync_once'"
            tone="blue"
            label="One-time sync"
            hint="Debezium snapshot only (snapshot.mode=initial_only). No ongoing WAL/binlog after the copy finishes."
          />
          <AppContextHint
            v-else-if="form.pattern === 'source_to_lake'"
            tone="brand"
            label="Event-first + Iceberg"
            hint="Debezium reads the source; IcebergSink writes from Kafka topics. This is not Analytics-first / OLake."
          />
          <AppContextHint
            v-else-if="form.pattern === 'source_to_olake'"
            tone="brand"
            label="Analytics-first"
            hint="OLake stack only — no Debezium, Kafka Connect, or JDBC replica. Save a draft. Start is blocked until Phase 3."
          />
          <AppContextHint
            v-else-if="form.pattern === 'source_to_stream'"
            tone="blue"
            label="Event-first"
            hint="Change events publish to Redpanda/Kafka topics. Start is Phase 2. Enable Iceberg from Kafka above for a lake landing."
          />
          <div v-if="form.pattern === 'source_to_target'" class="grid grid-cols-1 gap-4 md:grid-cols-2">
            <label class="md:col-span-2">
              <AppFormLabel
                label="Target name"
                hint="Optional — auto-named from host and database when empty."
              />
              <AppInput v-model="form.targetName" placeholder="Orders warehouse" />
            </label>
            <div class="md:col-span-2">
              <span class="auth-label">Target Type</span>
              <div class="grid grid-cols-4 gap-2 sm:grid-cols-4 lg:grid-cols-8">
                <button
                  v-for="t in dbTypes"
                  :key="t.value"
                  type="button"
                  class="flex flex-col items-center gap-1.5 rounded-xl border px-2 py-2.5 text-center transition-all hover:border-brand-500/60 hover:bg-brand-500/5"
                  :class="form.targetType === t.value
                    ? 'border-brand-500 bg-brand-500/10 text-foreground shadow-sm'
                    : 'border-border bg-card text-muted-foreground'"
                  @click="form.targetType = t.value as DatabaseType; form.targetPort = t.port"
                >
                  <img :src="t.icon" :alt="t.label" class="h-7 w-7 object-contain" />
                  <span class="text-xs font-medium leading-tight">{{ t.label }}</span>
                </button>
              </div>
            </div>
            <label>
              <span class="auth-label">Target Host</span>
              <AppInput v-model="form.targetHost" placeholder="cdc-postgres-target" />
            </label>
            <label>
              <span class="auth-label">Target Port</span>
              <AppInput v-model="form.targetPort" inputmode="numeric" placeholder="5432" />
            </label>
            <label>
              <span class="auth-label">Target Database</span>
              <AppInput v-model="form.targetDatabase" placeholder="demo" />
            </label>
            <div class="grid grid-cols-2 gap-3">
              <label>
                <span class="auth-label">Target Username</span>
                <AppInput v-model="form.targetUsername" placeholder="demo" />
              </label>
              <label>
                <span class="auth-label">Target Password</span>
                <AppInput
                  v-model="form.targetPassword"
                  type="password"
                  name="targetPassword"
                  autocomplete="current-password"
                  :placeholder="targetPasswordStored && !form.targetPassword ? '•••••• (stored)' : 'Target password'"
                  @input="targetPasswordStored = false"
                />
                <p v-if="targetPasswordStored && !form.targetPassword" class="mt-1 text-xs text-surface-400">
                  Password is saved. Leave blank to keep existing, or type to change it.
                </p>
              </label>
            </div>
            <label v-if="form.pattern === 'source_to_target'" class="flex items-start gap-2 text-sm">
              <input
                v-model="form.autostart"
                type="checkbox"
                class="mt-0.5 h-4 w-4 rounded border-border accent-brand-500"
              />
              <span>
                Autostart after orchestrator restart
                <span class="mt-0.5 block text-xs text-muted-foreground">
                  When running, reconcile again after redeploy or boot. Off = use Start manually.
                </span>
              </span>
            </label>
          </div>
          <div v-if="form.pattern === 'source_to_target'" class="space-y-4">
            <div class="flex flex-wrap items-end justify-between gap-3">
              <p class="text-sm text-muted-foreground">
                Connect to the target, then load schemas to preview tables that already exist.
                Table-by-table mapping (same name or rename) is the next step.
              </p>
              <AppButton :disabled="targetCatalogLoading || !canLoadTargetCatalog" @click="loadTargetCatalog">
                <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': targetCatalogLoading }" :stroke-width="1.75" />
                {{ targetCatalogLoading ? "Loading..." : "Load schemas" }}
              </AppButton>
            </div>
            <p v-if="targetCatalogError" class="text-sm text-red-700 dark:text-red-400">{{ targetCatalogError }}</p>
            <p v-else-if="!targetCatalogSchemas.length && !targetCatalogLoading" class="text-sm text-muted-foreground">
              Test the target connection or click Load schemas. Check one or more schemas below to list their tables.
            </p>
            <p
              v-else-if="targetCatalogSchemas.length && !targetSelectedSchemas.length"
              class="text-sm text-muted-foreground"
            >
              Select at least one schema to show its tables.
            </p>
            <div v-if="targetSchemaOptions.length" class="grid max-h-72 grid-cols-1 gap-2 overflow-y-auto md:grid-cols-2">
              <label
                v-for="schema in targetSchemaOptions"
                :key="`target-${schema}`"
                class="list-option flex cursor-pointer items-center gap-3"
                :class="{ 'list-option-active': isTargetSchemaSelected(schema) }"
              >
                <input
                  type="checkbox"
                  class="h-4 w-4 accent-brand-500"
                  :checked="isTargetSchemaSelected(schema)"
                  @change="toggleTargetSchema(schema)"
                />
                <span class="min-w-0">
                  <span class="block text-sm font-medium">{{ schema }}</span>
                  <span class="mt-0.5 block text-xs text-muted-foreground">{{ targetTableCountFor(schema) }} tables</span>
                </span>
              </label>
            </div>
            <div v-if="targetSelectedSchemas.length" class="space-y-3 rounded-xl border border-border bg-muted/40 p-4">
              <p class="text-sm font-medium">Tables in {{ targetSelectedSchemas.join(", ") }}</p>
              <AppInput v-model="targetTableSearch" placeholder="Filter tables..." />
              <p v-if="!targetCatalogTables.length && !targetCatalogLoading" class="text-sm text-muted-foreground">
                Load schemas to show tables for the selected schema.
              </p>
              <p v-else-if="!tablesForSelectedTargetSchemas.length" class="text-sm text-muted-foreground">
                No tables match this filter.
              </p>
              <div v-else class="max-h-96 space-y-3 overflow-auto pr-1">
                <div v-for="group in targetTablesGrouped" :key="group.schema" class="space-y-2">
                  <p class="text-xs font-medium text-muted-foreground">{{ group.schema }}</p>
                  <div
                    v-for="table in group.tables"
                    :key="`${table.schema}.${table.name}`"
                    class="flex items-center justify-between gap-3 rounded-xl border border-border px-3 py-2"
                  >
                    <span class="min-w-0">
                      <span class="block truncate font-mono text-sm">{{ table.name }}</span>
                      <span
                        class="mt-0.5 inline-flex items-center gap-1 text-xs"
                        :class="table.primary_key?.length ? 'text-muted-foreground' : 'text-yellow-700 dark:text-yellow-400'"
                      >
                        <KeyRound class="h-3 w-3" :stroke-width="1.75" />
                        {{ formatPrimaryKey(table.primary_key) }}
                      </span>
                    </span>
                    <span class="flex shrink-0 items-center gap-2">
                      <span class="text-right text-xs text-muted-foreground">{{ formatRecordCount(table.row_count) }}</span>
                      <AppButton size="sm" @click="inspectTable = { side: 'target', schema: table.schema, name: table.name }">
                        <Eye class="h-4 w-4" :stroke-width="1.75" />
                        View
                      </AppButton>
                    </span>
                  </div>
                </div>
              </div>
            </div>
          </div>
          <div v-else-if="form.pattern === 'source_to_lake'" class="grid grid-cols-1 gap-4 md:grid-cols-2">
            <label class="md:col-span-2">
              <span class="auth-label">Lake name</span>
              <AppInput v-model="form.lakeName" placeholder="analytics-lake" />
            </label>
            <label>
              <span class="auth-label">Table format</span>
              <AppSelect v-model="form.lakeFormat" aria-label="Lake table format">
                <option value="iceberg">Apache Iceberg</option>
                <option value="parquet">Parquet files</option>
              </AppSelect>
            </label>
            <label>
              <span class="auth-label">Zone</span>
              <AppSelect v-model="form.lakeZone" aria-label="Lake zone">
                <option value="bronze">Bronze — raw CDC changelog</option>
                <option value="silver">Silver — curated current table</option>
              </AppSelect>
            </label>
            <label>
              <span class="auth-label">Change mode</span>
              <AppSelect v-model="form.lakeChangeMode" aria-label="Change mode" :disabled="form.lakeZone === 'bronze'">
                <option value="changelog">Changelog (append)</option>
                <option value="upsert">Upsert</option>
                <option value="scd2">SCD Type 2</option>
              </AppSelect>
            </label>
            <label>
              <span class="auth-label">Catalog</span>
              <AppSelect v-model="form.lakeCatalog" aria-label="Iceberg catalog">
                <option value="jdbc">JDBC catalog</option>
                <option value="rest">REST catalog</option>
                <option value="nessie">Nessie</option>
                <option value="glue">AWS Glue</option>
                <option value="hadoop">Hadoop</option>
              </AppSelect>
            </label>
            <label>
              <span class="auth-label">Object storage endpoint</span>
              <AppInput v-model="form.lakeEndpoint" placeholder="http://cdc-lake:9000" />
            </label>
            <label>
              <span class="auth-label">Bucket</span>
              <AppInput v-model="form.lakeBucket" placeholder="lake" />
            </label>
            <label>
              <span class="auth-label">Region</span>
              <AppInput v-model="form.lakeRegion" placeholder="us-east-1" />
            </label>
            <label>
              <span class="auth-label">Warehouse</span>
              <AppInput v-model="form.lakeWarehouse" placeholder="s3://lake/warehouse" />
            </label>
            <label>
              <span class="auth-label">Access key</span>
              <AppInput v-model="form.lakeAccessKey" placeholder="cdcadmin" />
            </label>
            <label>
              <span class="auth-label">Secret key</span>
              <AppInput
                v-model="form.lakeSecretKey"
                type="password"
                name="lakeSecretKey"
                autocomplete="current-password"
                placeholder="Enter object-storage secret"
              />
            </label>
            <label v-if="form.lakeFormat === 'iceberg' && form.lakeCatalog !== 'hadoop' && form.lakeCatalog !== 'glue'" class="md:col-span-2">
              <span class="auth-label">Catalog URI</span>
              <AppInput v-model="form.lakeCatalogUri" placeholder="jdbc:postgresql://cdc-postgres-target:5432/demo" />
            </label>
            <label>
              <span class="auth-label">Domain</span>
              <AppInput v-model="form.domain" placeholder="commerce" />
            </label>
            <label>
              <span class="auth-label">Dataset</span>
              <AppInput v-model="form.dataset" placeholder="orders" />
            </label>
            <label>
              <span class="auth-label">Sensitivity</span>
              <AppSelect v-model="form.sensitivity" aria-label="Dataset sensitivity">
                <option value="public">Public</option>
                <option value="internal">Internal</option>
                <option value="confidential">Confidential</option>
                <option value="restricted">Restricted</option>
              </AppSelect>
            </label>
            <label>
              <span class="auth-label">Industry pack (optional)</span>
              <AppSelect v-model="form.industryPack" aria-label="Industry pack">
                <option value="">None — industry-agnostic</option>
                <option value="retail">Retail</option>
                <option value="banking">Banking</option>
                <option value="healthcare">Healthcare</option>
                <option value="manufacturing">Manufacturing</option>
                <option value="logistics">Logistics</option>
                <option value="telecom">Telecom</option>
                <option value="public_sector">Public sector</option>
              </AppSelect>
            </label>
          </div>
          <div v-else-if="form.pattern === 'source_to_stream'" class="grid grid-cols-1 gap-4 md:grid-cols-2">
            <label>
              <span class="auth-label">Stream Platform</span>
              <AppSelect v-model="form.streamPlatform" aria-label="Stream platform">
                <option value="redpanda">Redpanda</option>
                <option value="kafka">Apache Kafka</option>
              </AppSelect>
            </label>
          </div>
          <div class="flex flex-wrap items-center justify-between gap-3 border-t border-border pt-5">
            <div>
              <AppButton v-if="form.pattern === 'source_to_target' || form.pattern === 'source_to_lake'" :disabled="validating || !canTestTarget" @click="testTargetConnection">
                <CheckCircle2 class="h-4 w-4" :stroke-width="1.75" />
                {{ validating ? "Testing..." : form.pattern === "source_to_lake" ? "Test lake connection" : "Test Target Connection" }}
              </AppButton>
              <p
                v-if="targetValidation"
                class="mt-2 text-sm"
                :class="targetValidation.valid ? 'text-emerald-700 dark:text-emerald-400' : 'text-red-700 dark:text-red-400'"
              >
                {{ targetValidation.message }}
              </p>
            </div>
            <div class="flex gap-2">
              <AppButton @click="wizardBack">Back</AppButton>
              <AppButton variant="primary" :disabled="!canContinueTarget" @click="wizardNext">
                {{ form.pattern === "source_to_target" ? "Next: Mapping" : "Review pipeline" }}
                <ArrowRight class="h-4 w-4" :stroke-width="1.75" />
              </AppButton>
            </div>
          </div>
        </div>

        <div v-else-if="step === 'mapping'" class="space-y-5">
          <div>
            <AppSectionTitle
              title="Map tables"
              hint="After the target connection, map each selected source table to a target table. Keep the same name, or type a new name. Existing target tables appear in the list."
            />
          </div>

          <div v-if="!sourceTablesForMapping.length" class="rounded-xl border border-border bg-muted/40 p-4">
            <p class="text-sm font-medium">Same names for all tables</p>
            <p class="mt-1 text-sm text-muted-foreground">
              Source capture is set to all tables in the selected schema(s), so each target table keeps the source name.
              Pick specific tables on Source if you need to rename them one by one.
            </p>
            <div class="mt-3">
              <AppButton @click="goToWizardStep('source')">Choose specific tables</AppButton>
            </div>
          </div>

          <div v-else class="space-y-4">
            <div class="flex flex-wrap items-end justify-between gap-3">
              <label class="min-w-[220px] flex-1">
                <span class="auth-label">Target schema for all</span>
                <div class="flex gap-2">
                  <AppCombobox
                    v-model="mappingBulkSchema"
                    class="flex-1"
                    :options="mappingSchemaOptions"
                    placeholder="public"
                    aria-label="Target schema for all mapped tables"
                  />
                  <AppButton :disabled="!mappingBulkSchema.trim()" @click="applyTargetSchemaToAll">Apply</AppButton>
                </div>
              </label>
              <label class="min-w-[180px]">
                <span class="auth-label">Find table</span>
                <AppInput v-model="mappingSearch" placeholder="Filter source or target" />
              </label>
              <div class="flex flex-wrap gap-2">
                <AppButton :disabled="targetCatalogLoading || !canLoadTargetCatalog" @click="loadTargetCatalog">
                  <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': targetCatalogLoading }" :stroke-width="1.75" />
                  {{ targetCatalogLoading ? "Loading..." : "Refresh target tables" }}
                </AppButton>
                <AppButton @click="applySameNames">Use same names</AppButton>
                <AppButton
                  :disabled="!missingTargetMappings.length || targetCatalogLoading || !canLoadTargetCatalog"
                  title="Generate CREATE TABLE scripts (primary key, unique constraints and indexes) in the target database's syntax"
                  @click="openCreateTables(missingTargetMappings)"
                >
                  <Wand2 class="h-4 w-4" :stroke-width="1.75" />
                  Create missing tables{{ missingTargetMappings.length ? ` (${missingTargetMappings.length})` : "" }}
                </AppButton>
              </div>
            </div>
            <p v-if="targetCatalogError" class="text-sm text-red-700 dark:text-red-400">{{ targetCatalogError }}</p>
            <p v-else-if="targetCatalogLoading" class="text-sm text-muted-foreground">Loading target catalog…</p>
            <p class="text-xs text-muted-foreground">
              {{ form.mappings.length }} selected source table{{ form.mappings.length === 1 ? "" : "s" }}.
              {{ form.mappings.filter((mapping) => !mappingIsSameName(mapping)).length }} renamed.
              Type a new name or pick an existing target table. Status uses the live target catalog.
            </p>
              <AppTable>
              <thead>
                <tr>
                  <th>Source</th>
                  <th>Target schema</th>
                  <th>Target table</th>
                  <th>Status</th>
                  <th v-if="form.captureMode === 'trigger'" class="whitespace-nowrap">
                    Logical PK
                    <span class="ml-1 text-xs font-normal text-surface-400" title="Leave blank to auto-detect from target table PRIMARY KEY">(?)</span>
                  </th>
                  <th>Columns</th>
                </tr>
              </thead>
              <tbody>
                <template v-for="mapping in visibleMappings" :key="`${mapping.source_schema}.${mapping.source_table}`">
                  <tr>
                    <td>
                      <p class="font-medium">{{ mapping.source_table }}</p>
                      <p class="mt-0.5 font-mono text-xs text-muted-foreground">{{ mapping.source_schema }}</p>
                    </td>
                    <td>
                      <AppCombobox
                        v-model="mapping.target_schema"
                        :options="mappingSchemaOptions"
                        :placeholder="mapping.source_schema"
                        :aria-label="`Target schema for ${mapping.source_table}`"
                        @update:model-value="refreshMappingMode"
                      />
                    </td>
                    <td>
                      <AppCombobox
                        v-model="mapping.target_table"
                        :options="targetTablesForSchema(mapping.target_schema || mapping.source_schema).map((table) => table.name)"
                        :placeholder="mapping.source_table"
                        :aria-label="`Target table for ${mapping.source_table}`"
                        empty-text="No tables in this target schema yet — type a new name"
                        @update:model-value="refreshMappingMode"
                      />
                    </td>
                    <td>
                      <span :class="mappingStatus(mapping).tone">{{ mappingStatus(mapping).label }}</span>
                    </td>
                    <td v-if="form.captureMode === 'trigger'">
                      <AppInput
                        :model-value="(mapping.pk_columns ?? []).join(', ')"
                        placeholder="id  (auto-detected)"
                        :class="mappingStatus(mapping).label === 'No PK' ? 'border-yellow-500' : ''"
                        title="Comma-separated PK column names. Leave blank to auto-detect from target table PRIMARY KEY constraint."
                        @update:model-value="mapping.pk_columns = ($event as string).split(',').map((c: string) => c.trim()).filter(Boolean)"
                      />
                      <p
                        v-if="mappingStatus(mapping).label === 'No PK' && !(mapping.pk_columns ?? []).length"
                        class="mt-1 text-xs text-yellow-600 dark:text-yellow-400"
                      >
                        Target has no PK — enter columns above.
                      </p>
                    </td>
                    <td>
                      <div class="flex flex-wrap gap-2">
                        <button
                          type="button"
                          class="btn-secondary !h-8 !min-h-8 !px-3 !text-xs"
                          @click="openColMapper(mapping)"
                        >
                          <span v-if="!colInspectCache[colKey(mapping.source_schema, mapping.source_table)]?.expanded">
                            Map columns
                          </span>
                          <span v-else>Hide</span>
                        </button>
                        <button
                          type="button"
                          class="btn-secondary !h-8 !min-h-8 !px-3 !text-xs"
                          title="Compare source and target column structures"
                          @click="openStructureDiff(mapping)"
                        >
                          Compare
                        </button>
                        <button
                          v-if="mappingTargetMissing(mapping) && canLoadTargetCatalog"
                          type="button"
                          class="btn-secondary !h-8 !min-h-8 !px-3 !text-xs"
                          title="Script to create this table on the target with the source's primary key, unique constraints and indexes"
                          @click="openCreateTables([mapping])"
                        >
                          Create table
                        </button>
                      </div>
                    </td>
                  </tr>
                  <!-- Column mapper row (expandable) -->
                  <tr
                    v-if="colInspectCache[colKey(mapping.source_schema, mapping.source_table)]?.expanded"
                    :key="`col-mapper-${mapping.source_schema}.${mapping.source_table}`"
                  >
                    <td :colspan="form.captureMode === 'trigger' ? 6 : 5" class="p-4">
                      <p class="mb-3 text-xs font-semibold text-muted-foreground">
                        Column mapping: <span class="text-foreground">{{ mapping.source_schema }}.{{ mapping.source_table }}</span>
                        <span class="mx-1 text-brand-500">→</span>
                        <span class="text-foreground">{{ mapping.target_schema }}.{{ mapping.target_table }}</span>
                      </p>
                      <ColumnMapper
                        :source-columns="colInspectCache[colKey(mapping.source_schema, mapping.source_table)]?.cols ?? []"
                        :target-columns="colInspectCache[colKey(mapping.target_schema || mapping.source_schema, mapping.target_table || mapping.source_table)]?.cols ?? []"
                        :model-value="getColumnLinks(mapping)"
                        :loading="colInspectCache[colKey(mapping.source_schema, mapping.source_table)]?.loading ?? false"
                        :error="colInspectCache[colKey(mapping.source_schema, mapping.source_table)]?.error ?? ''"
                        @update:model-value="setColumnLinks(mapping, $event)"
                        @retry="() => { delete colInspectCache[colKey(mapping.source_schema, mapping.source_table)]; openColMapper(mapping) }"
                      />
                    </td>
                  </tr>
                </template>
              </tbody>
            </AppTable>
            <p v-if="!visibleMappings.length" class="text-sm text-muted-foreground">No tables match that filter.</p>
          </div>
          <div class="flex justify-between border-t border-border pt-5">
            <AppButton @click="wizardBack">Back</AppButton>
            <AppButton variant="primary" :disabled="!canContinueMapping" @click="wizardNext">
              Review pipeline <ArrowRight class="h-4 w-4" :stroke-width="1.75" />
            </AppButton>
          </div>
        </div>

        <div v-else class="space-y-5">
          <div>
            <AppSectionTitle title="Review pipeline" hint="Name the pipeline, then save a draft. Database replica can Enable or Start after save; Event-first and Analytics-first stay drafts in this phase." />
          </div>
          <div class="grid grid-cols-1 gap-4">
            <label>
              <span class="auth-label">Pipeline name</span>
              <AppInput v-model="form.name" placeholder="Orders CDC" />
            </label>
            <label>
              <span class="auth-label">Description</span>
              <textarea
                v-model="form.description"
                rows="3"
                class="auth-input form-textarea"
                placeholder="What this pipeline captures and where it writes."
              />
            </label>
            <div>
              <span class="auth-label">Tags</span>
              <PipelineTagInput v-model="form.tags" :suggestions="allTags" />
              <p class="mt-1 text-xs text-muted-foreground">
                Group pipelines in the list, e.g. by system, team or schedule. Press Enter or comma to add.
              </p>
            </div>
          </div>
          <div class="grid grid-cols-1 gap-4 md:grid-cols-3">
            <div class="rounded-xl border border-border bg-muted/50 p-4">
              <div class="mb-3 h-14 w-14 overflow-hidden rounded-xl bg-white">
                <img src="/images/cdc/pipeline-source.png" alt="Source" class="h-full w-full object-contain" />
              </div>
              <p class="text-sm font-medium">Source · {{ form.sourceType }}</p>
              <p class="mt-1 text-sm text-muted-foreground">{{ form.host }}:{{ form.port }}/{{ form.database }}</p>
              <p class="mt-1 text-xs text-muted-foreground">{{ captureCaption }}</p>
            </div>
            <div class="rounded-xl border border-border bg-muted/50 p-4">
              <div class="mb-3 h-14 w-14 overflow-hidden rounded-xl bg-white">
                <img src="/images/cdc/pipeline-transform.png" alt="Transform" class="h-full w-full object-contain" />
              </div>
              <div class="flex items-center gap-1">
                <p class="text-sm font-medium">Transform · {{ transformCaption }}</p>
                <AppInfoHint text="Optional SMT between source and target" />
              </div>
            </div>
            <div class="rounded-xl border border-border bg-muted/50 p-4">
              <div class="mb-3 h-14 w-14 overflow-hidden rounded-xl bg-white">
                <img src="/images/cdc/pipeline-target.png" alt="Target" class="h-full w-full object-contain" />
              </div>
              <p class="text-sm font-medium">
                Destination · {{
                  form.pattern === "source_to_olake"
                    ? "Iceberg (OLake)"
                    : form.pattern === "source_to_lake"
                    ? (form.lakeName || `${form.lakeZone} ${form.lakeFormat}`)
                    : form.pattern === "source_to_target"
                      ? (form.targetName || form.targetType)
                      : form.streamPlatform
                }}
              </p>
              <p class="mt-1 text-sm text-muted-foreground">
                {{
                  form.pattern === "source_to_olake"
                    ? "OLake stack — no Debezium on this source"
                    : form.pattern === "source_to_lake"
                    ? `${form.lakeEndpoint} · ${form.lakeBucket}`
                    : form.pattern === "source_to_target"
                      ? `${form.targetHost}:${form.targetPort}/${form.targetDatabase}`
                      : "Real-time change events"
                }}
              </p>
              <p class="mt-1 text-xs text-muted-foreground">{{ mappingCaption }}</p>
              <p v-if="form.pattern === 'source_to_target'" class="mt-1 text-xs text-muted-foreground">
                Rejected rows · {{ form.captureMode === "trigger" || form.sinkErrorPolicy === "dlq" ? "Dead-letter queue" : "Fail fast" }}
              </p>
            </div>
          </div>
          <div v-if="form.pattern === 'source_to_target'" class="rounded-xl border border-border bg-muted/40 p-4">
            <div class="flex items-center justify-between gap-3">
              <div class="flex items-center gap-1">
                <p class="text-sm font-medium">Table mapping</p>
                <AppInfoHint text="Same names as source when no custom mappings are defined." />
              </div>
              <AppButton @click="goToWizardStep('mapping')">Edit mapping</AppButton>
            </div>
            <ul v-if="form.mappings.length" class="mt-3 space-y-1.5">
              <li
                v-for="mapping in form.mappings"
                :key="`review-${mapping.source_schema}.${mapping.source_table}`"
                class="grid items-center gap-x-3 gap-y-1"
                style="grid-template-columns: 1fr auto 1fr auto"
              >
                <span class="truncate font-mono text-xs">{{ mapping.source_schema }}.{{ mapping.source_table }}</span>
                <ArrowRight class="h-3 w-3 shrink-0 text-muted-foreground" :stroke-width="1.75" />
                <span class="truncate font-mono text-xs">{{ mapping.target_schema }}.{{ mapping.target_table }}</span>
                <span :class="mappingStatus(mapping).tone">{{ mappingStatus(mapping).label }}</span>
              </li>
            </ul>
          </div>
          <AppContextHint
            v-if="form.pattern !== 'source_to_target'"
            tone="yellow"
            label="Start unavailable"
            :hint="`${startBlockedReason(form.pattern)} Save this draft. Only database replica can Start in this phase.`"
          />
          <div class="flex justify-between border-t border-border pt-5">
            <AppButton @click="wizardBack">Back</AppButton>
            <AppButton variant="primary" :disabled="saving || Boolean(dualCaptureWarning)" @click="save()">
              {{ saving ? "Saving..." : selectedId ? "Save and close" : "Save draft and close" }}
            </AppButton>
          </div>
        </div>
        </form>
      </section>

    <template v-else>
      <AppCard>
        <div class="grid grid-cols-1 gap-3 sm:grid-cols-4">
          <label class="sm:col-span-2">
            <span class="auth-label">Search</span>
            <div class="relative">
              <Search class="pointer-events-none absolute left-3 top-3 h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
              <AppInput v-model="pipelineSearch" input-class="pl-9" placeholder="Search name, tag, source, or target..." />
            </div>
          </label>
          <label>
            <span class="auth-label">Status</span>
            <AppSelect v-model="pipelineStatusFilter" aria-label="Filter pipelines by status">
              <option v-for="status in statusFilterOptions" :key="status" :value="status">
                {{ status === "all" ? "All statuses" : status }}
              </option>
            </AppSelect>
          </label>
          <label>
            <span class="auth-label">Tag</span>
            <AppSelect v-model="pipelineTagFilter" aria-label="Filter pipelines by tag">
              <option value="all">All tags</option>
              <option v-for="tag in allTags" :key="tag" :value="tag">{{ tag }}</option>
              <option :value="UNTAGGED">Untagged</option>
            </AppSelect>
          </label>
        </div>
      </AppCard>

      <AppCard>
        <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
          <p class="text-xs text-muted-foreground">
            {{ filteredProfiles.length }}{{ filteredProfiles.length !== profiles.length ? ` of ${profiles.length}` : "" }}
            pipeline{{ filteredProfiles.length === 1 ? "" : "s" }}
            <button
              v-if="pipelineTagFilter !== 'all'"
              type="button"
              class="ml-2 text-brand-600 hover:underline dark:text-brand-400"
              @click="pipelineTagFilter = 'all'"
            >
              Clear tag filter
            </button>
          </p>
          <div class="inline-flex rounded-lg border border-border bg-muted/40 p-0.5" role="tablist" aria-label="Pipeline view">
            <button
              v-for="mode in PIPELINE_VIEW_MODES"
              :key="mode.key"
              type="button"
              role="tab"
              :aria-selected="pipelineViewMode === mode.key"
              class="inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors"
              :class="pipelineViewMode === mode.key ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'"
              @click="pipelineViewMode = mode.key"
            >
              <component :is="mode.icon" class="h-3.5 w-3.5" :stroke-width="1.75" />
              <span class="hidden sm:inline">{{ mode.label }}</span>
            </button>
          </div>
        </div>

        <template v-if="pipelineViewMode === 'table'">
          <AppTable>
            <thead>
              <tr>
                <th
                  v-for="col in ([
                    { key: 'name', label: 'Name' },
                    { key: null, label: 'Source → target' },
                    { key: 'capture', label: 'Capture' },
                    { key: 'pattern', label: 'Pattern' },
                  ] as Array<{ key: PipelineSortKey | null; label: string }>)"
                  :key="col.label"
                  :aria-sort="col.key ? pipelineAriaSort(col.key) : undefined"
                >
                  <button
                    v-if="col.key"
                    type="button"
                    class="group/sort -mx-1 inline-flex items-center gap-1 rounded px-1 hover:text-foreground"
                    :class="pipelineSortKey === col.key ? 'text-foreground' : ''"
                    :title="`Sort by ${col.label.toLowerCase()}`"
                    @click="togglePipelineSort(col.key)"
                  >
                    {{ col.label }}
                    <component
                      :is="pipelineSortIcon(col.key)"
                      class="h-3.5 w-3.5 shrink-0"
                      :class="pipelineSortKey === col.key ? 'opacity-100' : 'opacity-40 group-hover/sort:opacity-80'"
                      :stroke-width="1.75"
                    />
                  </button>
                  <template v-else>{{ col.label }}</template>
                </th>
                <th>Status</th>
                <th class="w-28" />
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="profile in filteredProfiles"
                :key="profile.id"
                class="group cursor-pointer"
                :class="{ 'bg-brand-500/5': selectedId === profile.id }"
                @click="selectPipeline(profile.id)"
              >
                <td>
                  <div class="flex items-center gap-1.5">
                    <p class="font-medium">{{ profile.name }}</p>
                    <!-- Description sits behind an icon so long notes don't widen the table. -->
                    <AppInfoHint
                      v-if="profile.description"
                      :text="profile.description"
                      :aria-label="`About ${profile.name}`"
                    />
                  </div>
                  <div v-if="profile.tags?.length" class="mt-1 flex flex-wrap gap-1" @click.stop>
                    <button
                      v-for="tag in profile.tags"
                      :key="tag"
                      type="button"
                      class="tag-chip hover:bg-brand-500/20"
                      :title="`Show only '${tag}'`"
                      @click="pipelineTagFilter = tag"
                    >
                      {{ tag }}
                    </button>
                  </div>
                </td>
                <td>
                  <div class="flex items-center gap-1.5">
                    <AppDbIcon :type="profile.source.source_type" size="sm" class="opacity-90" />
                    <ArrowRight class="h-3 w-3 shrink-0 text-muted-foreground" :stroke-width="1.75" />
                    <AppDbIcon
                      v-if="profile.pattern === 'source_to_target'"
                      :type="profile.target?.target_type || 'postgres'"
                      size="sm"
                      class="opacity-90"
                    />
                    <Waves v-else class="h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
                  </div>
                  <p class="mt-1 font-mono text-xs text-muted-foreground">{{ pipelineRoute(profile) }}</p>
                </td>
                <td>{{ captureModeLabel(profile) }}</td>
                <td>{{ patternLabel(profile) }}</td>
                <td><AppStatusBadge :status="profile.status" /></td>
                <td>
                  <div class="flex items-center justify-end gap-1" @click.stop>
                    <button
                      type="button"
                      class="icon-button"
                      :title="profile.status === 'draft' ? 'Continue setup' : 'Edit'"
                      @click="openEdit(profile)"
                    >
                      <Pencil class="h-4 w-4" :stroke-width="1.75" />
                    </button>
                    <button
                      type="button"
                      class="icon-button text-red-500"
                      title="Delete"
                      @click="pendingDelete = profile"
                    >
                      <Trash2 class="h-4 w-4" :stroke-width="1.75" />
                    </button>
                  </div>
                </td>
              </tr>
              <tr v-if="loading && !profiles.length">
                <td colspan="6" class="py-6 text-center text-sm text-muted-foreground">Loading pipelines…</td>
              </tr>
              <tr v-else-if="!filteredProfiles.length">
                <td colspan="6">
                  <AppStateBlock
                    kind="empty"
                    :message="profiles.length ? 'No pipelines match this search.' : 'No pipelines configured yet. Use New Pipeline to add one.'"
                  />
                </td>
              </tr>
            </tbody>
          </AppTable>
        </template>

        <template v-else-if="pipelineViewMode === 'cards'">
          <div v-if="filteredProfiles.length" class="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
            <PipelineCard
              v-for="profile in filteredProfiles"
              :key="profile.id"
              :profile="profile"
              :route="pipelineRoute(profile)"
              :capture="captureModeLabel(profile)"
              :pattern="patternLabel(profile)"
              :selected="selectedId === profile.id"
              @select="selectPipeline(profile.id)"
              @edit="openEdit(profile)"
              @delete="pendingDelete = profile"
              @tag="pipelineTagFilter = $event"
            />
          </div>
          <AppStateBlock
            v-else
            :kind="loading && !profiles.length ? 'loading' : 'empty'"
            :message="loading && !profiles.length ? 'Loading pipelines…' : profiles.length ? 'No pipelines match this search.' : 'No pipelines configured yet. Use New Pipeline to add one.'"
          />
        </template>

        <template v-else-if="pipelineViewMode === 'tags'">
          <div v-if="profilesByTag.length" class="space-y-5">
            <section v-for="group in profilesByTag" :key="group.key">
              <h3 class="mb-2 flex items-center gap-2 text-sm font-semibold text-foreground">
                <Tags class="h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
                <button
                  v-if="group.key !== UNTAGGED"
                  type="button"
                  class="hover:underline"
                  :title="`Show only '${group.label}'`"
                  @click="pipelineTagFilter = group.key"
                >
                  {{ group.label }}
                </button>
                <span v-else class="text-muted-foreground">{{ group.label }}</span>
                <span class="badge-neutral">{{ group.items.length }}</span>
              </h3>
              <div class="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
                  <PipelineCard
                v-for="profile in group.items"
                :key="profile.id"
                :profile="profile"
                :route="pipelineRoute(profile)"
                :capture="captureModeLabel(profile)"
                :pattern="patternLabel(profile)"
                :selected="selectedId === profile.id"
                @select="selectPipeline(profile.id)"
                @edit="openEdit(profile)"
                @delete="pendingDelete = profile"
                @tag="pipelineTagFilter = $event"
              />
              </div>
            </section>
          </div>
          <AppStateBlock
            v-else
            :kind="loading && !profiles.length ? 'loading' : 'empty'"
            :message="loading && !profiles.length ? 'Loading pipelines…' : profiles.length ? 'No pipelines match this search.' : 'No pipelines configured yet. Use New Pipeline to add one.'"
          />
          <p v-if="profilesByTag.length && !allTags.length" class="mt-3 text-xs text-muted-foreground">
            No tags yet. Edit a pipeline and add tags on the first step to group it here.
          </p>
        </template>

        <template v-else-if="pipelineViewMode === 'capture'">
          <div v-if="profilesByCapture.length" class="space-y-5">
            <section v-for="group in profilesByCapture" :key="group.key">
              <h3 class="flex items-center gap-2 text-sm font-semibold text-foreground">
                <Radar class="h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
                {{ group.label }}
                <span class="badge-neutral">{{ group.items.length }}</span>
              </h3>
              <p class="mb-2 mt-0.5 text-xs text-muted-foreground">{{ group.description }}</p>
              <div class="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
                <PipelineCard
                  v-for="profile in group.items"
                  :key="profile.id"
                  :profile="profile"
                  :route="pipelineRoute(profile)"
                  :capture="captureModeLabel(profile)"
                  :pattern="patternLabel(profile)"
                  :selected="selectedId === profile.id"
                  @select="selectPipeline(profile.id)"
                  @edit="openEdit(profile)"
                  @delete="pendingDelete = profile"
                  @tag="pipelineTagFilter = $event"
                />
              </div>
            </section>
          </div>
          <AppStateBlock
            v-else
            :kind="loading && !profiles.length ? 'loading' : 'empty'"
            :message="loading && !profiles.length ? 'Loading pipelines…' : profiles.length ? 'No pipelines match this search.' : 'No pipelines configured yet. Use New Pipeline to add one.'"
          />
        </template>

        <PipelineScheduleView
          v-else
          :visible-ids="visibleProfileIds"
          :selected-id="selectedId"
          @select="selectPipeline"
        />
      </AppCard>

      <PipelineDetail
        v-if="selected && selectedFlow"
        :key="selected.id"
        :profile="selected"
        :flow="selectedFlow"
        :connectors="pipelineConnectors"
        :dlq-status="dlqStatus"
        :dlq-loading="dlqLoading"
        :can-start="canStartSelected"
        :start-blocked-reason="startBlockedReason(selected.pattern) || ''"
        :supports-test-sync="supportsCappedTableCopy"
        :busy="actionBusy"
        @action="onDetailAction"
        @autostart="persistAutostart"
        :tag-suggestions="allTags"
        :tags-saving="tagsSaving"
        @tags="persistTags"
      />
    </template>

    <AppConfirmDialog
      :open="startVerifyOpen"
      title="Test initial sync?"
      confirm-label="Run test & stop"
      :loading="startVerifying"
      @update:open="(open) => { if (!open && !startVerifying) startVerifyOpen = false }"
      @confirm="confirmStartVerify"
    >
      <p>
        Checks connectivity and configuration for
        <span class="font-semibold text-foreground">{{ selected?.name }}</span>: starts the pipeline, copies a
        <span class="font-medium">limited</span> row sample from each captured table into the target, then
        <span class="font-medium">stops it and removes its connectors</span>. Nothing keeps syncing — use
        Start for production.
      </p>
      <label class="mt-3 block text-sm">
        <span class="auth-label">Rows per table</span>
        <AppInput v-model="startVerifyRowLimit" inputmode="numeric" placeholder="100" />
      </label>
      <label class="mt-3 flex items-start gap-2 text-sm">
        <input
          v-model="startVerifyClearTarget"
          type="checkbox"
          class="mt-0.5 h-4 w-4 rounded border-border accent-brand-500"
        />
        <span>
          Empty target tables first
          <span class="block text-xs text-muted-foreground">Recommended for a clean verify run.</span>
        </span>
      </label>
      <p class="mt-2 text-xs text-muted-foreground">
        Trigger and incremental replica. Rows are the first pages returned from the source (not a random sample).
        Source offsets are reset on stop, so a later Start begins from the beginning.
      </p>
    </AppConfirmDialog>

    <AppConfirmDialog
      :open="resyncOpen"
      title="Resync from the source?"
      confirm-label="Resync"
      :loading="resyncing"
      @update:open="(open) => { if (!open && !resyncing) resyncOpen = false }"
      @confirm="confirmResync"
    >
      <p>
        <template v-if="selected?.source?.capture_mode === 'trigger'">
          Copies the current rows of the captured source tables into
          <span class="font-semibold text-foreground">{{ selected?.name }}</span>.
          Trigger capture only tails the changelog, so this is the way to load the full
          table. Ongoing changes continue from the current changelog position.
        </template>
        <template v-else>
          Replays every row of
          <span class="font-semibold text-foreground">{{ selected?.name }}</span>
          from the source. The pipeline is stopped, its source position is reset, and both
          connectors are resumed.
        </template>
      </p>
      <label class="mt-3 flex items-start gap-2 text-sm">
        <input v-model="resyncClearTarget" type="checkbox" class="mt-0.5 h-4 w-4 rounded border-border accent-brand-500" />
        <span>
          Empty the target tables first
          <span class="block text-xs text-muted-foreground">
            <template v-if="selected?.source?.capture_mode === 'trigger'">
              Truncates the replica, then copies every current source row. Leave this
              checked for a full refresh.
            </template>
            <template v-else>
              Removes rows that no longer exist in the source. Without this, existing rows are
              overwritten in place and stale deletes remain.
            </template>
          </span>
        </span>
      </label>
      <label v-if="supportsCappedTableCopy" class="mt-3 block text-sm">
        <span class="auth-label">Row limit (optional test load)</span>
        <AppInput
          v-model="resyncRowLimit"
          inputmode="numeric"
          placeholder="e.g. 100 — leave empty for full table"
        />
        <span class="mt-1 block text-xs text-muted-foreground">
          <template v-if="selected?.source?.capture_mode === 'incremental'">
            Incremental: capped direct copy only (does not reset JDBC offsets). Leave empty for a full resync (offset reset + replay).
          </template>
          <template v-else>
            Copies at most this many rows per captured table (first pages from the source, not a random sample).
          </template>
        </span>
      </label>
    </AppConfirmDialog>

    <SplitPipelineDialog
      :open="splitOpen"
      :profile="selected"
      @update:open="(open) => (splitOpen = open)"
      @done="load()"
      @open-created="openSplitResult"
      @restart-original="restartAfterSplit"
    />

    <DuplicatePipelineDialog
      :open="duplicateOpen"
      :profile="selected"
      @update:open="(open) => (duplicateOpen = open)"
      @done="load()"
      @open-created="openDuplicateResult"
    />

    <InitialLoadDialog
      :open="initialLoadOpen"
      :profile="selected"
      @update:open="(open) => (initialLoadOpen = open)"
      @finished="load()"
    />

    <AppConfirmDialog
      :open="Boolean(pendingDelete)"
      title="Delete this pipeline?"
      danger
      confirm-label="Delete"
      @update:open="(open) => { if (!open && !deleting) pendingDelete = null }"
      @confirm="confirmDelete"
    >
      <p>
        Delete <span class="font-semibold text-foreground">{{ pendingDelete?.name }}</span>?
        This removes the saved draft or configuration.
      </p>
    </AppConfirmDialog>

    <TriggerPreviewDialog
      :open="triggerPreviewTables.length > 0"
      :source="buildPayload().source"
      :tables="triggerPreviewTables"
      :installing="Boolean(triggerInstalling)"
      :install-error="triggerError"
      @update:open="(open) => { if (!open && !triggerInstalling) triggerPreviewTables = [] }"
      @confirm="confirmTriggerPreview"
    />
    <SourceTableInspectDialog
      v-if="inspectTable"
      :open="Boolean(inspectTable)"
      :source="inspectTable.side === 'source' ? buildPayload().source : undefined"
      :target="inspectTable.side === 'target' ? buildPayload().target || undefined : undefined"
      :schema="inspectTable.schema"
      :name="inspectTable.name"
      @update:open="(open) => { if (!open) inspectTable = null }"
    />
    <TableStructureDiffDialog
      v-if="structureDiff && buildPayload().target"
      :open="Boolean(structureDiff)"
      :source="buildPayload().source"
      :target="buildPayload().target!"
      :source-schema="structureDiff.source_schema"
      :source-table="structureDiff.source_table"
      :target-schema="structureDiff.target_schema"
      :target-table="structureDiff.target_table"
      :profile-id="selectedId || undefined"
      @update:open="(open) => { if (!open) structureDiff = null }"
    />
    <TargetTableDdlDialog
      v-if="ddlTables && buildPayload().target"
      :open="Boolean(ddlTables)"
      :source="buildPayload().source"
      :target="buildPayload().target!"
      :tables="ddlTables"
      :profile-id="selectedId || undefined"
      @created="loadTargetCatalog"
      @update:open="(open) => { if (!open) ddlTables = null }"
    />
  </div>
</template>
