<script setup lang="ts">
import {
  ArrowRight,
  CheckCircle2,
  Eye,
  Filter,
  KeyRound,
  Pause,
  Pencil,
  Play,
  Plus,
  RefreshCcwDot,
  RefreshCw,
  RotateCw,
  Save,
  ScrollText,
  Search,
  Square,
  TriangleAlert,
  Trash2,
  Waves,
  Zap,
} from "@lucide/vue"
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
  streamPlatform: "redpanda" as "kafka" | "redpanda",
  transformPreset: "none" as TransformPreset,
  deliveryMode: "realtime" as "realtime" | "micro_batch",
  intervalSeconds: "",
  fullRefreshMinutes: "",
  changelogRetentionHours: "",
  timestampColumn: "",
  incrementingColumn: "",
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
    (form.captureMode !== "incremental" || (filled(form.tables) && filled(form.incrementingColumn))) &&
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
const pipelineSearch = ref("")
const pipelineStatusFilter = ref("all")
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
    const result = await browseSourceCatalog({ ...buildPayload().source, profile_id: profileId })
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
  if (!canTestTarget.value) return
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
    const result = await browseSourceCatalog(buildPayload().source)
    applyCountUpdates(result.tables || [])
    if (result.schemas?.length) catalogSchemas.value = result.schemas
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

const buildPayload = (): ProfilePayload => ({
  name: textValue(form.name),
  description: textValue(form.description),
  pattern: form.pattern,
  stream_platform: form.pattern === "source_to_stream" ? form.streamPlatform : null,
  transform_strategy: form.transformPreset === "none" ? "none" : "predefined",
  sink_error_policy: form.sinkErrorPolicy,
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
    table_mode: form.captureMode === "trigger" || form.captureMode === "incremental" ? "specific_tables" : form.tableMode,
    capture_mode: form.captureMode,
    schemas: form.schemaMode === "specific_schemas" ? splitList(form.schemas) : [],
    tables: form.tableMode === "specific_tables" || form.captureMode === "trigger" || form.captureMode === "incremental" ? splitList(form.tables) : [],
    changelog_retention_hours:
      form.captureMode === "trigger" ? optionalPositiveInt(form.changelogRetentionHours) : null,
    timestamp_column: form.captureMode === "incremental" ? textValue(form.timestampColumn) || null : null,
    incrementing_column: form.captureMode === "incremental" ? textValue(form.incrementingColumn) || null : null,
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
    mappings:
      form.pattern === "source_to_target" &&
      // Identical-name mappings are normally dropped, but they must still be sent when
      // they carry a logical PK — trigger apply has no other source for those columns.
      (form.mappingMode === "custom" ||
        form.mappings.some((mapping) => (mapping.pk_columns ?? []).length > 0))
        ? form.mappings
            .filter((mapping) => filled(mapping.source_table) && filled(mapping.target_table))
            .map((mapping) => {
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
        : [],
  },
})

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
  })
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
    captureMode: (profile.source.capture_mode as CaptureMode) || "logical",
    sinkErrorPolicy: profile.sink_error_policy || "fail_fast",
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

const execute = async (action: "start" | "pause" | "resume" | "restart" | "stop") => {
  if (!selected.value) return
  if ((action === "start" || action === "restart") && !canStartSelected.value) {
    toast.warning(startBlockedReason(selected.value.pattern) || "Only replica pipelines can Start in this phase.")
    return
  }
  try {
    await runAction(selected.value.id, action)
    toast.success(`${action[0].toUpperCase()}${action.slice(1)} requested.`)
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Pipeline action failed")
  }
}

const resyncOpen = ref(false)
const resyncClearTarget = ref(true)
const resyncing = ref(false)

const confirmResync = async () => {
  if (!selected.value) return
  if (!canStartSelected.value) {
    toast.warning(startBlockedReason(selected.value.pattern))
    resyncOpen.value = false
    return
  }
  resyncing.value = true
  try {
    await resyncProfile(selected.value.id, resyncClearTarget.value)
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
const showProcessingLog = ref(false)

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
    showProcessingLog.value = false
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

const filteredProfiles = computed(() => {
  const query = pipelineSearch.value.trim().toLowerCase()
  const status = pipelineStatusFilter.value
  return profiles.value.filter((profile) => {
    if (status !== "all" && String(profile.status || "").toLowerCase() !== status) return false
    if (!query) return true
    const haystack = [
      profile.name,
      profile.description,
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
})

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
            <span class="auth-label">Pipeline name</span>
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
            <div
              v-if="form.captureMode === 'logical'"
              class="flex items-center gap-2 rounded-xl border border-emerald-500/25 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-700 dark:text-emerald-400"
            >
              <span class="font-medium">Logical CDC</span>
              <AppInfoHint text="Ongoing CDC. PostgreSQL sources need wal_level=logical. The demo stack already sets that." />
            </div>
            <div
              v-else-if="form.captureMode === 'trigger'"
              class="flex items-center gap-2 rounded-xl border border-yellow-500/30 bg-yellow-500/10 px-3 py-2 text-sm text-yellow-800 dark:text-yellow-300"
            >
              <span class="font-medium">Trigger capture</span>
              <AppInfoHint>
                Trigger mode does not use Debezium or WAL/binlog decoding. Select tables, create changelog triggers, then Start. The source connector is JDBC: it polls hyper_cdc.change_log (or the vendor equivalent). The source user needs CREATE TRIGGER. Native install runs on PostgreSQL, MySQL, MariaDB, SQL Server, and SQLite. Oracle catalog browsing is supported via DataService (requires ojdbc8.jar on DataService classpath). DB2 and Informix return the SQL to apply manually.
              </AppInfoHint>
            </div>
            <div
              v-else-if="form.captureMode === 'incremental'"
              class="flex items-center gap-2 rounded-xl border border-blue-500/25 bg-blue-500/10 px-3 py-2 text-sm text-blue-700 dark:text-blue-400"
            >
              <span class="font-medium">Incremental poll</span>
              <AppInfoHint text="Incremental polls the selected tables on a watermark column. No WAL, no triggers, no extra writes on the source. Use this for large tables. Hard deletes are not replicated — use Trigger on tables that must capture deletes. Watermark columns should be NOT NULL and indexed." />
            </div>
            <div
              v-else
              class="flex items-center gap-2 rounded-xl border border-blue-500/25 bg-blue-500/10 px-3 py-2 text-sm text-blue-700 dark:text-blue-400"
            >
              <span class="font-medium">One-time sync</span>
              <AppInfoHint text="One-time sync copies the selected tables once, then the pipeline stops capturing. Later inserts are not replicated until you run it again." />
            </div>
            <div v-if="form.captureMode === 'trigger'" class="grid grid-cols-1 gap-4 md:grid-cols-2">
              <label>
                <span class="auth-label">Poll cadence</span>
                <AppSelect v-model="form.deliveryMode" aria-label="Trigger poll cadence">
                  <option value="realtime">Realtime (every 1s)</option>
                  <option value="micro_batch">Interval (protect production)</option>
                </AppSelect>
              </label>
              <label v-if="form.deliveryMode === 'micro_batch'">
                <AppFormLabel
                  label="Interval (seconds)"
                  hint="JDBC polls only the changelog table, not your live tables. 300 = every 5 minutes."
                />
                <AppInput v-model="form.intervalSeconds" inputmode="numeric" placeholder="300" />
              </label>
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
                  hint="Optional if the table is append-only. Same column name on every selected table. Must be NOT NULL and indexed."
                />
                <AppInput v-model="form.timestampColumn" placeholder="updated_at" />
              </label>
              <label>
                <AppFormLabel
                  label="Incrementing / PK column"
                  hint="Required for replica upsert. Usually the primary key. Same column name on every selected table."
                />
                <AppInput v-model="form.incrementingColumn" placeholder="id" />
              </label>
              <label>
                <span class="auth-label">Poll cadence</span>
                <AppSelect v-model="form.deliveryMode" aria-label="Incremental poll cadence">
                  <option value="realtime">Realtime (every 1s)</option>
                  <option value="micro_batch">Interval (protect production)</option>
                </AppSelect>
              </label>
              <label v-if="form.deliveryMode === 'micro_batch'">
                <AppFormLabel
                  label="Interval (seconds)"
                  hint="JDBC reads only rows newer than the last watermark. 300 = every 5 minutes. Prefer this on large production tables."
                />
                <AppInput v-model="form.intervalSeconds" inputmode="numeric" placeholder="300" />
              </label>
              <label>
                <AppFormLabel label="Full refresh (minutes)">
                  Incremental capture only ever sees new rows. Set this to reload the whole table on a cycle so updates and deletes reach the target. Leave blank to disable. Costs a full table read each time — keep it well above the poll interval.
                </AppFormLabel>
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
            <p v-if="!sourceLiveCatalogSupported" class="rounded-xl border border-yellow-500/30 bg-yellow-500/10 p-3 text-sm text-yellow-800 dark:text-yellow-300">
              Live catalog browsing is not available for <span class="font-medium capitalize">{{ form.sourceType }}</span>. Enter schema and table names manually in the fields below.
              DB2 and Informix require manual entry.
            </p>
            <p v-else-if="form.schemaMode === 'all_schemas'" class="text-sm text-muted-foreground">
              Every schema and table in this database will be captured. Switch to Selected schemas to pick a schema and its tables.
            </p>
            <div v-else class="space-y-4">
              <p class="text-sm text-muted-foreground">
                <template v-if="form.captureMode === 'trigger'">
                  Select the tables to capture, then create a changelog trigger on each table. Trigger mode cannot capture every table in a schema without an explicit list.
                </template>
                <template v-else-if="form.captureMode === 'incremental'">
                  Select the large tables to poll. Incremental cannot capture every table in a schema without an explicit list. Prefer tables that share the same timestamp and PK column names.
                </template>
                <template v-else>
                  Select one or more schemas, then check the source tables to capture. Leave tables unchecked to capture every table in the selected schema.
                </template>
              </p>
              <p v-if="catalogError" class="text-sm text-red-700 dark:text-red-400">{{ catalogError }}</p>
              <p v-else-if="!catalogSchemas.length && !catalogLoading" class="text-sm text-muted-foreground">
                Test the connection or click Load schemas, then pick the schemas to capture. Nothing is selected until you check a schema.
              </p>

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
                <p class="text-xs text-muted-foreground">
                  {{ selectedTableKeys.length ? `${selectedTableKeys.length} table(s) selected` : form.captureMode === "trigger" ? "Select tables, then create a trigger on each one" : form.captureMode === "incremental" ? "Select tables to poll on the watermark column" : "All tables in the selected schema(s) will be captured" }}
                  <span v-if="lastCountLabel"> · {{ lastCountLabel }}</span>
                </p>
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
          <div
            v-if="dualCaptureWarning"
            class="rounded-xl border border-red-500/30 bg-red-500/10 p-4 text-sm text-red-700 dark:text-red-400"
          >
            {{ dualCaptureWarning }}
          </div>
          <div
            v-if="form.pattern === 'source_to_target' && form.captureMode === 'logical'"
            class="rounded-xl border border-emerald-500/25 bg-emerald-500/10 p-4 text-sm text-emerald-700 dark:text-emerald-400"
          >
            Replica uses the capture mode chosen on Source. CDC needs <span class="font-medium">wal_level=logical</span>. Leave Transform as None for a straight copy.
          </div>
          <div
            v-else-if="form.pattern === 'source_to_target' && form.captureMode === 'trigger'"
            class="rounded-xl border border-yellow-500/30 bg-yellow-500/10 p-4 text-sm text-yellow-800 dark:text-yellow-300"
          >
            Trigger replica uses a JDBC source on the changelog table, not Debezium. Create triggers on Source before Start. The JDBC sink still upserts into the target.
          </div>
          <div
            v-else-if="form.pattern === 'source_to_target' && form.captureMode === 'incremental'"
            class="rounded-xl border border-blue-500/25 bg-blue-500/10 p-4 text-sm text-blue-700 dark:text-blue-400"
          >
            Incremental replica uses JDBC source on the live tables (not Debezium, not triggers). The sink upserts on the PK column. Hard deletes on the source are not applied on the target.
          </div>
          <div
            v-else-if="form.pattern === 'source_to_target' && form.captureMode === 'sync_once'"
            class="rounded-xl border border-blue-500/25 bg-blue-500/10 p-4 text-sm text-blue-700 dark:text-blue-400"
          >
            One-time sync uses Debezium snapshot only (<span class="font-medium">snapshot.mode=initial_only</span>). No ongoing WAL/binlog after the copy finishes.
          </div>
          <div
            v-else-if="form.pattern === 'source_to_lake'"
            class="rounded-xl border border-brand-500/25 bg-brand-500/10 p-4 text-sm text-brand-800 dark:text-brand-300"
          >
            Event-first plus Iceberg from Kafka. Debezium still reads the source; IcebergSink writes from topics. This is not Analytics-first / OLake.
          </div>
          <div
            v-else-if="form.pattern === 'source_to_olake'"
            class="rounded-xl border border-brand-500/25 bg-brand-500/10 p-4 text-sm text-brand-800 dark:text-brand-300"
          >
            Analytics-first uses the OLake stack only — no Debezium, Kafka Connect, or JDBC replica. Save a draft. Start is blocked until Phase 3.
          </div>
          <div
            v-else
            class="rounded-xl border border-blue-500/25 bg-blue-500/10 p-4 text-sm text-blue-700 dark:text-blue-400"
          >
            Change events are published to Redpanda/Kafka topics. Start is Phase 2. Add Iceberg from Kafka above if you also want a lake landing.
          </div>
          <div v-if="form.pattern === 'source_to_target'" class="grid grid-cols-1 gap-4 md:grid-cols-2">
            <label class="md:col-span-2">
              <span class="auth-label">Target Name <span class="font-normal text-muted-foreground">(optional — auto-named from host/database)</span></span>
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
          </div>
          <div v-if="form.pattern === 'source_to_target'" class="space-y-4">
            <div class="flex flex-wrap items-end justify-between gap-3">
              <p class="text-sm text-muted-foreground">
                Connect to the target, then load schemas to preview tables that already exist.
                Table-by-table mapping (same name or rename) is the next step.
              </p>
              <AppButton :disabled="targetCatalogLoading || !canTestTarget" @click="loadTargetCatalog">
                <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': targetCatalogLoading }" :stroke-width="1.75" />
                {{ targetCatalogLoading ? "Loading..." : "Load schemas" }}
              </AppButton>
            </div>
            <p v-if="targetCatalogError" class="text-sm text-red-700 dark:text-red-400">{{ targetCatalogError }}</p>
            <p v-else-if="!targetCatalogSchemas.length && !targetCatalogLoading" class="text-sm text-muted-foreground">
              Test the target connection or click Load schemas, then pick a schema to list its tables.
            </p>
            <div v-if="targetSchemaOptions.length" class="grid grid-cols-1 gap-2 md:grid-cols-2">
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
                  <AppInput
                    v-model="mappingBulkSchema"
                    list="mapping-schema-options"
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
                <AppButton :disabled="targetCatalogLoading || !canTestTarget" @click="loadTargetCatalog">
                  <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': targetCatalogLoading }" :stroke-width="1.75" />
                  {{ targetCatalogLoading ? "Loading..." : "Refresh target tables" }}
                </AppButton>
                <AppButton @click="applySameNames">Use same names</AppButton>
              </div>
            </div>
            <datalist id="mapping-schema-options">
              <option v-for="schema in mappingSchemaOptions" :key="`schema-opt-${schema}`" :value="schema" />
            </datalist>
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
                      <AppInput
                        v-model="mapping.target_schema"
                        list="mapping-schema-options"
                        :placeholder="mapping.source_schema"
                        :aria-label="`Target schema for ${mapping.source_table}`"
                        @update:model-value="refreshMappingMode"
                      />
                    </td>
                    <td>
                      <AppInput
                        v-model="mapping.target_table"
                        :list="`target-tables-${mapping.source_schema}-${mapping.source_table}`"
                        :placeholder="mapping.source_table"
                        @update:model-value="refreshMappingMode"
                      />
                      <datalist :id="`target-tables-${mapping.source_schema}-${mapping.source_table}`">
                        <option v-for="table in targetTablesForSchema(mapping.target_schema)" :key="table.name" :value="table.name" />
                      </datalist>
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
          <div
            v-if="form.pattern !== 'source_to_target'"
            class="rounded-xl border border-yellow-500/30 bg-yellow-500/10 p-4 text-sm text-yellow-800 dark:text-yellow-300"
          >
            {{ startBlockedReason(form.pattern) }} Save this draft. Only database replica can Start in this phase.
          </div>
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
        <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <label class="sm:col-span-2">
            <span class="auth-label">Search</span>
            <div class="relative">
              <Search class="pointer-events-none absolute left-3 top-3 h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
              <AppInput v-model="pipelineSearch" input-class="pl-9" placeholder="Search name, source, or target..." />
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
        </div>
      </AppCard>

      <AppCard>
        <div class="mb-3 text-xs text-muted-foreground">
          {{ filteredProfiles.length }}{{ filteredProfiles.length !== profiles.length ? ` of ${profiles.length}` : "" }}
          pipeline{{ filteredProfiles.length === 1 ? "" : "s" }}
        </div>
        <AppTable>
          <thead>
            <tr>
              <th>Name</th>
              <th>Source → target</th>
              <th>Capture</th>
              <th>Pattern</th>
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
                <p class="font-medium">{{ profile.name }}</p>
                <p v-if="profile.description" class="mt-0.5 truncate text-xs text-muted-foreground">
                  {{ profile.description }}
                </p>
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
      </AppCard>

      <section v-if="selected" class="space-y-4">
        <div class="stat-card">
          <div class="page-toolbar">
            <div>
              <div class="flex items-center gap-2">
                <h2 class="text-lg font-semibold text-foreground">{{ selected.name }}</h2>
                <AppStatusBadge :status="selected.status" />
              </div>
              <AppInfoHint
                :text="selected.description || `${selected.source.source_type} → ${destinationSummary(selected)}`"
              />
            </div>
            <div class="flex flex-wrap gap-2">
              <AppButton variant="primary" @click="editSelected">
                {{ selected.status === "draft" ? "Continue setup" : "Edit Configuration" }}
              </AppButton>
              <AppButton
                :variant="selected.enabled ? 'destructive' : 'primary'"
                :disabled="!selected.enabled && !canStartSelected"
                @click="toggleEnabled"
              >
                {{ selected.enabled ? "Disable" : "Enable" }}
              </AppButton>
              <AppButton variant="destructive" @click="pendingDelete = selected">Delete</AppButton>
            </div>
          </div>
        </div>
        <PipelineFlow
          variant="viewer"
          :pattern="selected.pattern"
          :status="selected.status"
          :source-label="databaseLabel(selected.source.source_type)"
          :source-caption="connectionCaption(selected.source.connection)"
          :source-image="dbIconPath(selected.source.source_type)"
          :transform-caption="viewerTransformCaption(selected)"
          :target-label="destinationSummary(selected)"
          :target-caption="viewerTargetCaption(selected)"
          :target-image="selected.pattern === 'source_to_target' ? dbIconPath(selected.target?.target_type) : ''"
        />
        <div class="grid grid-cols-1 gap-4 md:grid-cols-4">
          <AppMetricCard label="Source" :value="selected.source.source_type" :helper="selected.source.connection.host || '—'" tone="brand">
            <AppDbIcon :type="selected.source.source_type" size="sm" />
          </AppMetricCard>
          <AppMetricCard label="Delivery" :value="selected.flow.schedule.delivery_mode" helper="Change event schedule" tone="info">
            <Waves class="h-5 w-5" :stroke-width="1.75" />
          </AppMetricCard>
          <AppMetricCard label="Capture Scope" :value="selected.source.tables.length || 'All'" helper="Selected tables" tone="neutral">
            <CheckCircle2 class="h-5 w-5" :stroke-width="1.75" />
          </AppMetricCard>
          <AppMetricCard
            label="Sink Errors"
            :value="selected.pattern === 'source_to_stream' || selected.pattern === 'source_to_olake' ? 'No sink' : dlqLoading ? 'Loading…' : dlqStatus?.enabled ? `${dlqStatus.messages_count} rejected` : 'Fail fast'"
            :helper="selected.pattern === 'source_to_olake' ? 'OLake does not use a Kafka Connect sink' : selected.pattern === 'source_to_stream' ? 'Event-first pipeline' : dlqError || dlqStatus?.topic || 'Sink stops on rejected records'"
            tone="warning"
          >
            <TriangleAlert class="h-5 w-5" :stroke-width="1.75" />
          </AppMetricCard>
        </div>
        <div class="stat-card">
          <h2 class="section-title mb-4">Pipeline Controls</h2>
          <div class="flex flex-wrap gap-2">
            <AppButton variant="primary" :disabled="!canStartSelected" @click="execute('start')"><Play class="h-4 w-4" />Start</AppButton>
            <AppButton @click="execute('pause')"><Pause class="h-4 w-4" />Pause</AppButton>
            <AppButton @click="execute('resume')"><Play class="h-4 w-4" />Resume</AppButton>
            <AppButton :disabled="!canStartSelected" @click="execute('restart')"><RotateCw class="h-4 w-4" />Restart</AppButton>
            <AppButton @click="execute('stop')"><Square class="h-4 w-4" />Stop</AppButton>
            <AppButton :disabled="!canStartSelected" @click="resyncOpen = true"><RefreshCcwDot class="h-4 w-4" />Resync</AppButton>
            <AppButton
              :variant="showProcessingLog ? 'primary' : 'secondary'"
              @click="showProcessingLog = !showProcessingLog"
            >
              <ScrollText class="h-4 w-4" />{{ showProcessingLog ? "Hide Log" : "View Log" }}
            </AppButton>
          </div>
          <p class="mt-3 text-xs text-muted-foreground">
            <template v-if="canStartSelected">
              Resync replays every row from the source. Use it after clearing target data —
              a plain Restart resumes where the source left off and will not re-send old rows.
            </template>
            <template v-else>{{ startBlockedReason(selected.pattern) }}</template>
          </p>
        </div>

        <!-- Per-pipeline processing log -->
        <PipelineProcessingLog
          v-if="showProcessingLog"
          :key="selected.id"
          :connectors="pipelineConnectors"
          :title="`Processing Log — ${selected.name}`"
        />
      </section>
    </template>

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
    </AppConfirmDialog>

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
  </div>
</template>
