<script setup lang="ts">
import { Database, Download, X } from "@lucide/vue"
import DatabaseTypeMark from "~/components/connectors/DatabaseTypeMark.vue"
import type { ConnectorTemplate } from "~/composables/useConnectorsM2"
import { DATABASE_TYPES, insightFromConfig, stringifyConfig, type ConnectorKind, type ConnectorDatabaseType } from "~/utils/connectorMeta"

const props = defineProps<{
  open: boolean
  templates: ConnectorTemplate[]
  templatesLoading: boolean
  templatesError: string
  actionLoading: boolean
}>()

const emit = defineEmits<{
  close: []
  created: [name: string]
}>()

const { validateConnector, createConnector, normalizeApiError } = useConnectorsM2()
const toast = useToast()

const step = ref(1)
const createKind = ref<ConnectorKind>("source")
const createDbType = ref<ConnectorDatabaseType>("postgres")
const createName = ref("")
const createConfigText = ref("")
const validateResult = ref<any>(null)
const createValidateHash = ref("")
const error = ref("")

const createConfigHash = computed(() => `${createName.value.trim()}\u0000${createConfigText.value}`)
const canCreate = computed(
  () =>
    Boolean(createName.value.trim()) &&
    validateResult.value?.valid === true &&
    createValidateHash.value === createConfigHash.value
)
const matchingTemplates = computed(() =>
  props.templates.filter((template) => {
    const insight = insightFromConfig(template.config)
    const kind = template.kind || insight.kind
    const db = template.database_type || insight.databaseType
    return kind === createKind.value && db === createDbType.value
  })
)
const previewInsight = computed(() => {
  try {
    return insightFromConfig(JSON.parse(createConfigText.value || "{}"))
  } catch {
    return insightFromConfig({})
  }
})

const parseJsonObject = (text: string) => {
  const value = JSON.parse(text)
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("JSON must be an object")
  }
  return Object.fromEntries(Object.entries(value as Record<string, unknown>).map(([key, val]) => [key, String(val)]))
}

const applyBestTemplate = () => {
  const template = matchingTemplates.value[0]
  if (template) {
    createConfigText.value = stringifyConfig(template.config)
    if (!createName.value) createName.value = `${template.id}-${Date.now()}`
    return
  }
  const port = DATABASE_TYPES.find((item) => item.value === createDbType.value)?.port || "5432"
  if (createKind.value === "sink") {
    const jdbc =
      createDbType.value === "mysql" || createDbType.value === "mariadb"
        ? `jdbc:mysql://db:${port}/demo`
        : createDbType.value === "sqlserver"
          ? `jdbc:sqlserver://db:${port};databaseName=demo`
          : createDbType.value === "oracle"
            ? `jdbc:oracle:thin:@//db:${port}/demo`
            : `jdbc:postgresql://db:${port}/demo?currentSchema=public`
    createConfigText.value = stringifyConfig({
      "connector.class": "io.debezium.connector.jdbc.JdbcSinkConnector",
      "tasks.max": "1",
      "topics.regex": "demo\\..*",
      "connection.url": jdbc,
      "connection.username": "demo",
      "connection.password": "demo",
      "insert.mode": "upsert",
      "delete.enabled": "true",
      "primary.key.mode": "record_key",
      "schema.evolution": "basic",
    })
    return
  }
  const className =
    createDbType.value === "mysql" || createDbType.value === "mariadb"
      ? "io.debezium.connector.mysql.MySqlConnector"
      : createDbType.value === "sqlserver"
        ? "io.debezium.connector.sqlserver.SqlServerConnector"
        : createDbType.value === "oracle"
          ? "io.debezium.connector.oracle.OracleConnector"
          : "io.debezium.connector.postgresql.PostgresConnector"
  createConfigText.value = stringifyConfig({
    "connector.class": className,
    "tasks.max": "1",
    "topic.prefix": "demo",
    "database.hostname": "db",
    "database.port": port,
    "database.user": "demo",
    "database.password": "demo",
    "database.dbname": "demo",
  })
}

const reset = () => {
  step.value = 1
  createKind.value = "source"
  createDbType.value = "postgres"
  createName.value = ""
  createConfigText.value = ""
  validateResult.value = null
  createValidateHash.value = ""
  error.value = ""
}

watch(
  () => props.open,
  (open) => {
    if (open) reset()
  }
)

watch([createKind, createDbType], () => {
  if (step.value >= 3) applyBestTemplate()
})

watch([createName, createConfigText], () => {
  if (createValidateHash.value !== createConfigHash.value) validateResult.value = null
})

const goConfig = () => {
  applyBestTemplate()
  step.value = 3
}

const runValidate = async () => {
  error.value = ""
  try {
    const config = parseJsonObject(createConfigText.value)
    validateResult.value = await validateConnector({
      name: createName.value.trim() || undefined,
      config,
    })
    createValidateHash.value = createConfigHash.value
    if (validateResult.value.valid) toast.success("Validation passed.")
    else toast.warning("Validation found issues.")
  } catch (e: any) {
    validateResult.value = null
    createValidateHash.value = ""
    error.value = normalizeApiError(e, "Validation failed")
  }
}

const runCreate = async () => {
  error.value = ""
  try {
    const name = createName.value.trim()
    if (!name) throw new Error("Connector name is required")
    if (!canCreate.value) throw new Error("Validate this exact name and configuration first.")
    const config = parseJsonObject(createConfigText.value)
    await createConnector({ name, config })
    toast.success(`Connector ${name} created.`)
    emit("created", name)
  } catch (e: any) {
    error.value = normalizeApiError(e, "Create failed")
  }
}
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="modal-overlay"
      role="dialog"
      aria-modal="true"
      aria-labelledby="create-connector-title"
      @click.self="emit('close')"
    >
      <section class="auth-card flex max-h-[90vh] w-full max-w-3xl flex-col overflow-hidden bg-card p-5 animate-slide-up">
        <div class="mb-4 flex items-center justify-between gap-3">
          <div>
            <h2 id="create-connector-title" class="section-title">New Connector</h2>
            <AppInfoHint text="Choose Source or Sink, then a database type, then review JSON." />
          </div>
          <AppButton size="icon" title="Close" @click="emit('close')">
            <X class="h-4 w-4" :stroke-width="1.75" />
          </AppButton>
        </div>

        <div class="mb-4 flex items-center gap-2">
          <template v-for="item in 3" :key="item">
            <div
              :class="[
                'flex h-8 w-8 items-center justify-center rounded-full border text-sm font-semibold',
                step >= item ? 'border-primary bg-primary text-primary-foreground' : 'border-border bg-muted text-muted-foreground',
              ]"
            >
              {{ item }}
            </div>
            <div v-if="item < 3" class="h-px flex-1" :class="step > item ? 'bg-primary' : 'bg-border'" />
          </template>
        </div>

        <div class="min-h-[520px] flex-1 overflow-y-auto">
          <div v-if="step === 1" class="grid grid-cols-1 gap-3 md:grid-cols-2">
            <button
              type="button"
              class="list-option p-5"
              :class="createKind === 'source' ? 'list-option-active' : ''"
              @click="createKind = 'source'"
            >
              <Download class="mb-3 h-6 w-6 text-blue-600 dark:text-blue-400" :stroke-width="1.75" />
              <p class="section-title">Source</p>
              <p class="mt-1 text-sm text-muted-foreground">Capture change events from a database into Kafka topics.</p>
            </button>
            <button
              type="button"
              class="list-option p-5"
              :class="createKind === 'sink' ? 'list-option-active' : ''"
              @click="createKind = 'sink'"
            >
              <Database class="mb-3 h-6 w-6 text-emerald-600 dark:text-emerald-400" :stroke-width="1.75" />
              <p class="section-title">Sink</p>
              <p class="mt-1 text-sm text-muted-foreground">Apply those events into a target database.</p>
            </button>
          </div>

          <div v-else-if="step === 2" class="grid grid-cols-2 gap-3 md:grid-cols-3">
            <button
              v-for="db in DATABASE_TYPES"
              :key="db.value"
              type="button"
              class="list-option p-4"
              :class="createDbType === db.value ? 'list-option-active' : ''"
              @click="createDbType = db.value"
            >
              <DatabaseTypeMark :database-type="db.value" :kind="createKind" size="lg" :show-kind="false" />
              <p class="mt-3 text-sm text-muted-foreground">Default port {{ db.port }}</p>
            </button>
          </div>

          <div v-else class="space-y-3">
            <DatabaseTypeMark
              :database-type="previewInsight.databaseType"
              :kind="previewInsight.kind"
              size="lg"
              :subtitle="`${previewInsight.connection.host || 'host'}:${previewInsight.connection.port || 'port'}/${previewInsight.connection.database || 'database'}`"
            />
            <p v-if="templatesError" class="alert-warning">{{ templatesError }}</p>
            <label>
              <span class="auth-label">Connector Name</span>
              <AppInput v-model="createName" placeholder="orders-postgres-source" />
            </label>
            <label class="block">
              <span class="auth-label">JSON Config</span>
              <textarea v-model="createConfigText" class="auth-input form-textarea" spellcheck="false" />
            </label>
            <div v-if="validateResult" class="text-sm">
              <p>
                Valid:
                <span :class="validateResult.valid ? 'text-emerald-700 dark:text-emerald-400' : 'text-red-700 dark:text-red-400'">
                  {{ validateResult.valid }}
                </span>
              </p>
              <p v-if="validateResult.warnings?.length" class="mt-1 text-amber-700 dark:text-amber-400">
                Warnings: {{ validateResult.warnings.join(" | ") }}
              </p>
              <p v-if="validateResult.errors?.length" class="mt-1 text-red-700 dark:text-red-400">
                Errors:
                <span v-for="(item, idx) in validateResult.errors" :key="idx">{{ item.field }} - {{ item.message }}; </span>
              </p>
            </div>
            <AppStateBlock v-if="error" kind="error" :message="error" />
          </div>
        </div>

        <div class="mt-4 flex items-center justify-between border-t border-border pt-4">
          <AppButton v-if="step > 1" :disabled="actionLoading" @click="step -= 1">Back</AppButton>
          <span v-else />
          <div class="flex gap-2">
            <template v-if="step === 1">
              <AppButton variant="primary" @click="step = 2">Continue</AppButton>
            </template>
            <template v-else-if="step === 2">
              <AppButton variant="primary" :disabled="templatesLoading" @click="goConfig">Continue</AppButton>
            </template>
            <template v-else>
              <AppButton :disabled="actionLoading" @click="runValidate">Validate</AppButton>
              <AppButton variant="primary" :disabled="actionLoading || !canCreate" @click="runCreate">Create</AppButton>
            </template>
          </div>
        </div>
      </section>
    </div>
  </Teleport>
</template>
