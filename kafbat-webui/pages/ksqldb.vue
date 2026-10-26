<script setup lang="ts">
import { Play, RefreshCw, Square, Trash2, X, Zap } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"

const { fetchKsqlInfo, fetchKsqlTables, fetchKsqlStreams, fetchKsqlQueries, runKsql, terminateKsqlQuery, dropKsqlEntity, explainKsqlQuery } = useKafkaAdmin()
const toast = useToast()

const tab = ref<"tables" | "streams" | "queries" | "editor">("tables")
const loading = ref(false)
const error = ref("")
const configured = ref(true)
const reachable = ref(true)
const ksqlVersion = ref("")
const tables = ref<Array<Record<string, unknown>>>([])
const streams = ref<Array<Record<string, unknown>>>([])
const queries = ref<Array<Record<string, unknown>>>([])

const SNIPPETS = [
  "SHOW STREAMS;",
  "SHOW TABLES;",
  "SHOW QUERIES;",
  "SHOW TOPICS;",
  "PRINT 'demo.bookings.bookings' FROM BEGINNING LIMIT 1;",
  "CREATE STREAM cdc_raw WITH (KAFKA_TOPIC='demo.bookings.airplanes_data', VALUE_FORMAT='AVRO');",
  "SELECT * FROM cdc_raw EMIT CHANGES LIMIT 10;",
]

const ksqlText = ref("SHOW STREAMS;")
const running = ref(false)
const queryResult = ref<any>(null)
const queryError = ref("")

// Drop confirm
const pendingDrop = ref<{ type: "STREAM" | "TABLE"; name: string } | null>(null)
const deleteTopic = ref(false)
const dropping = ref(false)

// Terminate confirm
const pendingTerminate = ref<string | null>(null)
const terminating = ref(false)

// Explain panel
const explainQueryId = ref<string | null>(null)
const explainResult = ref<any>(null)
const explainLoading = ref(false)

const load = async () => {
  loading.value = true
  error.value = ""
  try {
    const [infoData, tableData, streamData, queryData] = await Promise.allSettled([
      fetchKsqlInfo(),
      fetchKsqlTables(),
      fetchKsqlStreams(),
      fetchKsqlQueries(),
    ])
    if (infoData.status === "fulfilled") {
      configured.value = infoData.value.configured !== false
      reachable.value = infoData.value.reachable !== false
      const info = infoData.value.info as { KsqlServerInfo?: { version?: string } } | undefined
      ksqlVersion.value = info?.KsqlServerInfo?.version || ""
    }
    if (tableData.status === "fulfilled") {
      tables.value = tableData.value.items || []
      configured.value = tableData.value.configured !== false
    } else {
      error.value = formatApiError(tableData.reason, "Failed to load ksqlDB")
    }
    if (streamData.status === "fulfilled") {
      streams.value = streamData.value.items || []
    }
    if (queryData.status === "fulfilled") {
      queries.value = queryData.value.items || []
    }
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load ksqlDB")
  } finally {
    loading.value = false
  }
}

const execute = async () => {
  if (!ksqlText.value.trim()) return
  running.value = true
  queryResult.value = null
  queryError.value = ""
  try {
    const result = await runKsql(ksqlText.value)
    queryResult.value = result
    const upper = ksqlText.value.toUpperCase()
    if (upper.includes("CREATE") || upper.includes("DROP") || upper.includes("TERMINATE") || upper.startsWith("SHOW")) {
      await load()
    }
  } catch (e: any) {
    queryError.value = formatApiError(e, "ksqlDB query failed")
  } finally {
    running.value = false
  }
}

const queryTable = computed(() => {
  const result = queryResult.value
  if (!result || typeof result !== "object") return null
  if (result.kind === "query" && Array.isArray(result.rows)) {
    const columns = Array.isArray(result.columns) && result.columns.length
      ? result.columns as string[]
      : (result.rows[0] ? Object.keys(result.rows[0]) : [])
    return { columns, rows: result.rows as Array<Record<string, unknown>> }
  }
  if (Array.isArray(result) && result.length && typeof result[0] === "object") {
    return { columns: Object.keys(result[0]), rows: result as Array<Record<string, unknown>> }
  }
  if (result.kind === "statement" && Array.isArray(result.result) && result.result.length) {
    return { columns: Object.keys(result.result[0]), rows: result.result as Array<Record<string, unknown>> }
  }
  return null
})

const confirmDrop = async () => {
  if (!pendingDrop.value) return
  const { type, name } = pendingDrop.value
  pendingDrop.value = null
  dropping.value = true
  try {
    await dropKsqlEntity(type, name, deleteTopic.value)
    toast.success(`Dropped ${type.toLowerCase()} "${name}"`)
    deleteTopic.value = false
    await load()
  } catch (e: any) {
    toast.error(formatApiError(e, `Failed to drop ${type.toLowerCase()}`))
  } finally {
    dropping.value = false
  }
}

const confirmTerminate = async () => {
  if (!pendingTerminate.value) return
  const qid = pendingTerminate.value
  pendingTerminate.value = null
  terminating.value = true
  try {
    await terminateKsqlQuery(qid)
    toast.success(`Terminated query ${qid}`)
    await load()
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to terminate query"))
  } finally {
    terminating.value = false
  }
}

const openExplain = async (queryId: string) => {
  explainQueryId.value = queryId
  explainResult.value = null
  explainLoading.value = true
  try {
    explainResult.value = await explainKsqlQuery(queryId)
  } catch (e: any) {
    explainResult.value = { error: formatApiError(e, "Failed to explain query") }
  } finally {
    explainLoading.value = false
  }
}

// Insert snippet into editor
const insertSnippet = (s: string) => {
  ksqlText.value = s
  tab.value = "editor"
}

const tableColumns = computed(() => {
  const first = tables.value[0]
  return first ? Object.keys(first) : []
})
const streamColumns = computed(() => {
  const first = streams.value[0]
  return first ? Object.keys(first) : []
})
const queryColumns = computed(() => {
  const first = queries.value[0]
  return first ? Object.keys(first) : []
})

// Get entity name from a row
const entityName = (row: Record<string, unknown>) =>
  String(row.name || row.NAME || row.streamName || row.tableName || "")

const queryId = (row: Record<string, unknown>) =>
  String(row.id || row.queryId || row.queryID || row["@type"] || "")

onMounted(load)
</script>

<template>
  <div class="page-container max-w-7xl">
    <AppPageHeader title="KSQL DB" description="Query CDC Avro topics with ksqlDB — streams, tables, and persistent queries.">
      <AppButton :disabled="loading" @click="load">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton variant="primary" @click="tab = 'editor'">
        <Zap class="h-4 w-4" :stroke-width="1.75" />
        Execute KSQL
      </AppButton>
    </AppPageHeader>

    <!-- Metrics -->
    <section class="grid grid-cols-2 gap-3 sm:grid-cols-4">
      <AppMetricCard label="Tables" :value="String(tables.length)" tone="info">
        <RefreshCw class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard label="Streams" :value="String(streams.length)" tone="brand">
        <Play class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard label="Queries" :value="String(queries.length)" tone="success">
        <Zap class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Server"
        :value="!configured ? 'Off' : reachable ? (ksqlVersion || 'Up') : 'Down'"
        :tone="!configured || !reachable ? undefined : 'success'"
      />
    </section>

    <AppStateBlock v-if="error" kind="error" :message="error" />
    <AppStateBlock
      v-else-if="!configured"
      kind="empty"
      message="ksqlDB is not configured. Set ORCH_KSQL_URL and start the cdc-ksqldb container."
    />
    <AppStateBlock
      v-else-if="!reachable"
      kind="error"
      message="ksqlDB is configured but not reachable. Wait for cdc-ksqldb to become healthy, then refresh."
    />

    <!-- Tabs -->
    <div class="flex flex-wrap gap-1 rounded-xl border border-surface-700 bg-surface-900/30 p-1">
      <button
        v-for="t in [
          { id: 'tables', label: `Tables (${tables.length})` },
          { id: 'streams', label: `Streams (${streams.length})` },
          { id: 'queries', label: `Queries (${queries.length})` },
          { id: 'editor', label: 'KSQL Editor' },
        ]"
        :key="t.id"
        type="button"
        :class="[
          'rounded-lg px-4 py-2 text-sm font-medium transition-colors',
          tab === t.id ? 'bg-surface-800 text-surface-50 shadow-sm' : 'text-surface-400 hover:text-surface-200',
        ]"
        @click="tab = t.id as typeof tab"
      >
        {{ t.label }}
      </button>
    </div>

    <!-- Tables tab -->
    <AppCard v-if="tab === 'tables'">
      <div class="mb-3 flex items-center justify-between">
        <h2 class="section-title">Tables</h2>
        <AppButton size="sm" @click="insertSnippet('CREATE TABLE my_table WITH (KAFKA_TOPIC=\'my-topic\', VALUE_FORMAT=\'AVRO\') AS SELECT * FROM my_stream EMIT CHANGES;')">
          + Create Table
        </AppButton>
      </div>
      <AppTable>
        <thead>
          <tr>
            <th v-for="col in tableColumns" :key="col">{{ col }}</th>
            <th class="w-20" />
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, idx) in tables" :key="idx" class="group">
            <td v-for="col in tableColumns" :key="col" class="font-mono text-sm">{{ row[col] }}</td>
            <td>
              <div class="invisible flex items-center justify-end gap-1 group-hover:visible">
                <button
                  type="button"
                  title="Drop table"
                  class="icon-button text-red-500 hover:bg-red-500/10"
                  @click="pendingDrop = { type: 'TABLE', name: entityName(row) }"
                >
                  <Trash2 class="h-3.5 w-3.5" :stroke-width="1.75" />
                </button>
              </div>
            </td>
          </tr>
          <tr v-if="!tables.length">
            <td :colspan="tableColumns.length + 1"><AppStateBlock kind="empty" message="No tables found." /></td>
          </tr>
        </tbody>
      </AppTable>
    </AppCard>

    <!-- Streams tab -->
    <AppCard v-else-if="tab === 'streams'">
      <div class="mb-3 flex items-center justify-between">
        <h2 class="section-title">Streams</h2>
        <AppButton size="sm" @click="insertSnippet('CREATE STREAM my_stream WITH (KAFKA_TOPIC=\'my-topic\', VALUE_FORMAT=\'AVRO\');')">
          + Create Stream
        </AppButton>
      </div>
      <AppTable>
        <thead>
          <tr>
            <th v-for="col in streamColumns" :key="col">{{ col }}</th>
            <th class="w-20" />
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, idx) in streams" :key="idx" class="group">
            <td v-for="col in streamColumns" :key="col" class="font-mono text-sm">{{ row[col] }}</td>
            <td>
              <div class="invisible flex items-center justify-end gap-1 group-hover:visible">
                <button
                  type="button"
                  title="Drop stream"
                  class="icon-button text-red-500 hover:bg-red-500/10"
                  @click="pendingDrop = { type: 'STREAM', name: entityName(row) }"
                >
                  <Trash2 class="h-3.5 w-3.5" :stroke-width="1.75" />
                </button>
              </div>
            </td>
          </tr>
          <tr v-if="!streams.length">
            <td :colspan="streamColumns.length + 1"><AppStateBlock kind="empty" message="No streams found." /></td>
          </tr>
        </tbody>
      </AppTable>
    </AppCard>

    <!-- Queries tab -->
    <AppCard v-else-if="tab === 'queries'">
      <div class="mb-3">
        <h2 class="section-title">Persistent Queries</h2>
        <p class="mt-0.5 text-xs text-surface-400">Queries created with CREATE TABLE AS SELECT or CREATE STREAM AS SELECT.</p>
      </div>
      <AppTable>
        <thead>
          <tr>
            <th>Query ID</th>
            <th>State</th>
            <th v-for="col in queryColumns.filter(c => !['id','queryId','queryID','state','STATE'].includes(c))" :key="col">{{ col }}</th>
            <th class="w-24" />
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, idx) in queries" :key="idx" class="group">
            <td class="font-mono text-sm font-medium">{{ queryId(row) }}</td>
            <td>
              <span
                :class="[
                  'rounded-md px-2 py-0.5 text-xs font-medium',
                  String(row.state || row.STATE || '').toUpperCase() === 'RUNNING'
                    ? 'bg-emerald-500/10 text-emerald-400'
                    : 'bg-surface-700/50 text-surface-400',
                ]"
              >
                {{ row.state || row.STATE || '—' }}
              </span>
            </td>
            <td
              v-for="col in queryColumns.filter(c => !['id','queryId','queryID','state','STATE'].includes(c))"
              :key="col"
              class="font-mono text-sm text-surface-300"
            >
              {{ row[col] }}
            </td>
            <td>
              <div class="invisible flex items-center justify-end gap-1 group-hover:visible">
                <button
                  type="button"
                  title="Explain query"
                  class="icon-button hover:bg-blue-500/10 hover:text-blue-400"
                  @click="openExplain(queryId(row))"
                >
                  <Zap class="h-3.5 w-3.5" :stroke-width="1.75" />
                </button>
                <button
                  type="button"
                  title="Terminate query"
                  class="icon-button text-red-500 hover:bg-red-500/10"
                  @click="pendingTerminate = queryId(row)"
                >
                  <Square class="h-3.5 w-3.5" :stroke-width="1.75" />
                </button>
              </div>
            </td>
          </tr>
          <tr v-if="!queries.length">
            <td colspan="10"><AppStateBlock kind="empty" message="No persistent queries." /></td>
          </tr>
        </tbody>
      </AppTable>
    </AppCard>

    <!-- KSQL Editor tab -->
    <AppCard v-else-if="tab === 'editor'" class="space-y-4">
      <div class="flex items-center justify-between">
        <h2 class="section-title">KSQL Editor</h2>
        <div class="flex flex-wrap gap-1">
          <button
            v-for="snippet in SNIPPETS"
            :key="snippet"
            type="button"
            class="rounded-lg border border-surface-700 px-2 py-1 text-xs text-surface-400 hover:border-surface-500 hover:text-surface-200"
            @click="ksqlText = snippet"
          >
            {{ snippet.length > 42 ? snippet.slice(0, 40) + '…' : snippet }}
          </button>
        </div>
      </div>
      <textarea
        v-model="ksqlText"
        rows="10"
        class="auth-input font-mono text-sm"
        spellcheck="false"
        placeholder="SHOW TABLES;"
      />
      <div class="flex items-center gap-3">
        <AppButton variant="primary" :disabled="running || !ksqlText.trim()" @click="execute">
          <Play class="h-4 w-4" :stroke-width="1.75" />
          {{ running ? "Executing…" : "Execute" }}
        </AppButton>
        <AppButton v-if="queryResult || queryError" @click="queryResult = null; queryError = ''">
          <X class="h-4 w-4" :stroke-width="1.75" />
          Clear
        </AppButton>
      </div>

      <!-- Result -->
      <div v-if="queryError" class="alert-error">{{ queryError }}</div>
      <div v-if="queryResult">
        <div class="mb-2 text-xs font-medium text-surface-400">
          Result
          <span v-if="queryTable"> · {{ queryTable.rows.length }} row(s)</span>
        </div>
        <AppTable v-if="queryTable" class="text-sm">
          <thead>
            <tr>
              <th v-for="col in queryTable.columns" :key="col">{{ col }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(row, i) in queryTable.rows" :key="i">
              <td v-for="col in queryTable.columns" :key="col" class="font-mono">
                {{ typeof row[col] === 'object' ? JSON.stringify(row[col]) : row[col] }}
              </td>
            </tr>
            <tr v-if="!queryTable.rows.length">
              <td :colspan="Math.max(queryTable.columns.length, 1)">
                <AppStateBlock kind="empty" message="Query returned no rows." />
              </td>
            </tr>
          </tbody>
        </AppTable>
        <pre v-else class="code-block max-h-[360px] overflow-auto whitespace-pre-wrap text-sm">{{ JSON.stringify(queryResult, null, 2) }}</pre>
      </div>
    </AppCard>

    <!-- Explain panel (Teleport modal) -->
    <Teleport to="body">
      <div
        v-if="explainQueryId"
        class="modal-overlay"
        role="dialog"
        aria-modal="true"
        @click.self="explainQueryId = null"
      >
        <section class="auth-card flex max-h-[85vh] w-full max-w-3xl flex-col overflow-hidden animate-slide-up">
          <div class="mb-3 flex items-center justify-between gap-3">
            <h2 class="section-title">Explain: {{ explainQueryId }}</h2>
            <button type="button" class="icon-button" @click="explainQueryId = null">
              <X class="h-4 w-4" :stroke-width="1.75" />
            </button>
          </div>
          <div class="flex-1 overflow-auto">
            <AppStateBlock v-if="explainLoading" kind="loading" message="Loading explanation…" />
            <pre v-else class="code-block whitespace-pre-wrap text-sm">{{ JSON.stringify(explainResult, null, 2) }}</pre>
          </div>
        </section>
      </div>
    </Teleport>

    <!-- Drop stream/table confirm -->
    <AppConfirmDialog
      :open="Boolean(pendingDrop)"
      :title="`Drop ${pendingDrop?.type?.toLowerCase() ?? 'entity'}?`"
      danger
      confirm-label="Drop"
      @update:open="(v) => { if (!v) pendingDrop = null }"
      @confirm="confirmDrop"
    >
      <div class="space-y-3 text-sm">
        <p>
          Drop {{ pendingDrop?.type?.toLowerCase() }}
          <strong class="font-mono">{{ pendingDrop?.name }}</strong>?
        </p>
        <label class="flex items-center gap-2 cursor-pointer">
          <input v-model="deleteTopic" type="checkbox" class="rounded border-surface-600" />
          <span class="text-surface-300">Also delete the underlying Kafka topic</span>
        </label>
      </div>
    </AppConfirmDialog>

    <!-- Terminate query confirm -->
    <AppConfirmDialog
      :open="Boolean(pendingTerminate)"
      title="Terminate query?"
      danger
      confirm-label="Terminate"
      @update:open="(v) => { if (!v) pendingTerminate = null }"
      @confirm="confirmTerminate"
    >
      Terminate persistent query <strong class="font-mono">{{ pendingTerminate }}</strong>?
    </AppConfirmDialog>
  </div>
</template>
