<script setup lang="ts">
import {
  ArrowDown,
  ArrowUp,
  Braces,
  Database,
  Filter,
  Plus,
  RefreshCw,
  Save,
  Sparkles,
  Trash2,
} from "@lucide/vue"
import type { ProcessingPipeline, ProcessingStage } from "~/composables/useProcessingPipelines"
import { formatApiError } from "~/utils/apiError"

const api = useProcessingPipelines()
const toast = useToast()
const pipelines = ref<ProcessingPipeline[]>([])
const selectedId = ref("")
const loading = ref(false)
const saving = ref(false)
const error = ref("")
const validationErrors = ref<string[]>([])

const newId = () => crypto.randomUUID()
const emptyPipeline = (): ProcessingPipeline => ({
  id: newId(),
  name: "",
  input_topic: "",
  output_topic: "",
  dead_letter_topic: "",
  consumer_group: "",
  batch: { max_records: 500, max_bytes: 4_194_304, max_wait_ms: 25 },
  stages: [],
  preserve_partition_order: true,
  enabled: false,
})
const draft = ref<ProcessingPipeline>(emptyPipeline())

const stageIcon = (stage: ProcessingStage) =>
  stage.kind === "transform" ? Braces : stage.kind === "filter" ? Filter : Database

const load = async () => {
  loading.value = true
  error.value = ""
  try {
    pipelines.value = await api.fetchPipelines()
    if (selectedId.value) {
      const selected = pipelines.value.find((item) => item.id === selectedId.value)
      if (selected) draft.value = structuredClone(selected)
    }
  } catch (cause: any) {
    error.value = formatApiError(cause, "Failed to load processing pipelines")
  } finally {
    loading.value = false
  }
}

const selectPipeline = (pipeline: ProcessingPipeline) => {
  selectedId.value = pipeline.id
  draft.value = structuredClone(pipeline)
  validationErrors.value = []
}

const createNew = () => {
  selectedId.value = ""
  draft.value = emptyPipeline()
  validationErrors.value = []
}

const addStage = (kind: ProcessingStage["kind"]) => {
  const index = draft.value.stages.filter((stage) => stage.kind === kind).length + 1
  draft.value.stages.push({
    id: `${kind}-${index}`,
    name: `${kind.charAt(0).toUpperCase()}${kind.slice(1)} ${index}`,
    kind,
    execution: kind === "enrich" ? "data_service" : "grpc",
    endpoint: kind === "enrich" ? null : "http://127.0.0.1:50051",
    dataservice:
      kind === "enrich"
        ? {
            base_url: "http://127.0.0.1:8080",
            datasource: "",
            mapper: "",
            api_key_env: "DATA_SERVICE_API_KEY",
            lookup_field: "customer_id",
            lookup_parameter: "list",
            result_key_field: "customer_id",
            output_field: "enrichment",
            static_parameters: {},
          }
        : null,
    timeout_ms: 10_000,
    retry: { max_attempts: 3, initial_backoff_ms: 100, max_backoff_ms: 5_000 },
    failure_policy: "dead_letter",
    enabled: true,
    attributes: {},
  })
}

const setExecution = (stage: ProcessingStage, execution: ProcessingStage["execution"]) => {
  stage.execution = execution
  if (execution === "grpc") {
    stage.endpoint ||= "http://127.0.0.1:50051"
    stage.dataservice = null
  } else {
    stage.endpoint = null
    stage.dataservice ||= {
      base_url: "http://127.0.0.1:8080",
      datasource: "",
      mapper: "",
      api_key_env: "DATA_SERVICE_API_KEY",
      lookup_field: "customer_id",
      lookup_parameter: "list",
      result_key_field: "customer_id",
      output_field: "enrichment",
      static_parameters: {},
    }
  }
}

const moveStage = (index: number, direction: -1 | 1) => {
  const target = index + direction
  if (target < 0 || target >= draft.value.stages.length) return
  const [stage] = draft.value.stages.splice(index, 1)
  draft.value.stages.splice(target, 0, stage)
}

const validate = async () => {
  const result = await api.validatePipeline(draft.value)
  validationErrors.value = result.errors || []
  return result.valid
}

const save = async () => {
  saving.value = true
  error.value = ""
  try {
    if (!(await validate())) {
      error.value = validationErrors.value.join(". ")
      return
    }
    const stored = selectedId.value
      ? await api.updatePipeline(draft.value)
      : await api.createPipeline(draft.value)
    selectedId.value = stored.id
    draft.value = structuredClone(stored)
    toast.success("Processing pipeline saved.")
    await load()
  } catch (cause: any) {
    error.value = formatApiError(cause, "Failed to save processing pipeline")
  } finally {
    saving.value = false
  }
}

const removePipeline = async () => {
  if (!selectedId.value || !confirm(`Delete ${draft.value.name || "this pipeline"}?`)) return
  try {
    await api.deletePipeline(selectedId.value)
    toast.success("Processing pipeline deleted.")
    createNew()
    await load()
  } catch (cause: any) {
    error.value = formatApiError(cause, "Failed to delete processing pipeline")
  }
}

onMounted(load)
</script>

<template>
  <div class="page-container">
    <AppPageHeader
      title="Event Processing"
      description="Configure high-throughput Transform, Filter, and Hydrate / Enrich stages between raw and trusted Kafka topics."
    >
      <AppButton variant="secondary" :disabled="loading" @click="load">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" />
        Refresh
      </AppButton>
      <AppButton variant="primary" @click="createNew">
        <Plus class="h-4 w-4" />
        New Pipeline
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <section class="grid grid-cols-1 gap-5 xl:grid-cols-[280px,minmax(0,1fr)]">
      <aside class="stat-card h-fit">
        <div class="mb-3 flex items-center justify-between">
          <h2 class="section-title">Pipelines</h2>
          <AppStatusBadge :status="`${pipelines.length} configured`" />
        </div>
        <AppEmptyState
          v-if="!pipelines.length"
          title="No Processing Pipelines"
          description="Create the first raw-to-trusted event flow."
        >
          <template #icon><Sparkles class="h-5 w-5" /></template>
        </AppEmptyState>
        <div v-else class="space-y-2">
          <button
            v-for="pipeline in pipelines"
            :key="pipeline.id"
            type="button"
            class="list-option"
            :class="{ 'list-option-active': selectedId === pipeline.id }"
            @click="selectPipeline(pipeline)"
          >
            <div class="flex items-center justify-between gap-2">
              <span class="truncate text-sm font-medium">{{ pipeline.name }}</span>
              <AppStatusBadge :status="pipeline.enabled ? 'Enabled' : 'Draft'" />
            </div>
            <p class="mt-1 truncate font-mono text-sm text-muted-foreground">
              {{ pipeline.input_topic }} → {{ pipeline.output_topic }}
            </p>
          </button>
        </div>
      </aside>

      <div class="space-y-5">
        <section class="stat-card">
          <div class="mb-4">
            <AppSectionTitle title="Pipeline Contract" hint="Raw input, trusted output, replay consumer group, and bounded batching." />
          </div>
          <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
            <label>
              <span class="auth-label">Name</span>
              <AppInput v-model="draft.name" placeholder="Customer Trusted View" />
            </label>
            <label>
              <span class="auth-label">Consumer Group</span>
              <AppInput v-model="draft.consumer_group" placeholder="hyper-cdc-customers-v1" />
            </label>
            <label>
              <span class="auth-label">Raw Input Topic</span>
              <AppInput v-model="draft.input_topic" placeholder="cdc.customers.raw" />
            </label>
            <label>
              <span class="auth-label">Trusted Output Topic</span>
              <AppInput v-model="draft.output_topic" placeholder="cdc.customers.trusted.v1" />
            </label>
            <label>
              <span class="auth-label">Dead-Letter Topic</span>
              <AppInput v-model="draft.dead_letter_topic" placeholder="cdc.customers.dlq" />
            </label>
            <label class="flex items-end gap-2 pb-2 text-sm text-foreground">
              <input v-model="draft.preserve_partition_order" type="checkbox" class="h-4 w-4 accent-brand-500" />
              Preserve Kafka partition order
            </label>
            <label>
              <span class="auth-label">Maximum Records per Batch</span>
              <AppInput v-model="draft.batch.max_records" type="number" />
            </label>
            <label>
              <span class="auth-label">Maximum Wait (ms)</span>
              <AppInput v-model="draft.batch.max_wait_ms" type="number" />
            </label>
          </div>
        </section>

        <section class="stat-card">
          <div class="mb-4 flex flex-wrap items-center justify-between gap-3">
            <div>
              <AppSectionTitle title="Processing Stages" hint="Stages execute in order. Dropped records do not continue to later stages." />
            </div>
            <div class="flex flex-wrap gap-2">
              <AppButton @click="addStage('transform')"><Braces class="h-4 w-4" />Transform</AppButton>
              <AppButton @click="addStage('filter')"><Filter class="h-4 w-4" />Filter</AppButton>
              <AppButton @click="addStage('enrich')"><Database class="h-4 w-4" />Enrich</AppButton>
            </div>
          </div>

          <AppEmptyState
            v-if="!draft.stages.length"
            title="No Processing Stages"
            description="Add Transform, Filter, or Enrich. An empty chain passes records through unchanged."
          >
            <template #icon><Sparkles class="h-5 w-5" /></template>
          </AppEmptyState>

          <div v-else class="space-y-3">
            <article v-for="(stage, index) in draft.stages" :key="`${stage.id}-${index}`" class="rounded-xl border border-border bg-muted/30 p-4">
              <div class="mb-4 flex items-center justify-between gap-3">
                <div class="flex items-center gap-3">
                  <component :is="stageIcon(stage)" class="h-5 w-5 text-primary" />
                  <div>
                    <div class="flex items-center gap-1">
                      <p class="text-sm font-semibold">{{ index + 1 }}. {{ stage.name || stage.kind }}</p>
                      <AppInfoHint :text="stage.execution === 'grpc' ? 'Remote batched gRPC' : 'Hyper DataService mapper'" />
                    </div>
                  </div>
                </div>
                <div class="flex gap-1">
                  <AppButton size="icon" title="Move Up" :disabled="index === 0" @click="moveStage(index, -1)"><ArrowUp class="h-4 w-4" /></AppButton>
                  <AppButton size="icon" title="Move Down" :disabled="index === draft.stages.length - 1" @click="moveStage(index, 1)"><ArrowDown class="h-4 w-4" /></AppButton>
                  <AppButton size="icon" variant="destructive" title="Remove Stage" @click="draft.stages.splice(index, 1)"><Trash2 class="h-4 w-4" /></AppButton>
                </div>
              </div>

              <div class="grid grid-cols-1 gap-3 md:grid-cols-3">
                <label><span class="auth-label">Stage ID</span><AppInput v-model="stage.id" /></label>
                <label><span class="auth-label">Display Name</span><AppInput v-model="stage.name" /></label>
                <label>
                  <span class="auth-label">Execution</span>
                  <AppSelect :model-value="stage.execution" @update:model-value="setExecution(stage, $event as ProcessingStage['execution'])">
                    <option value="grpc">Remote gRPC</option>
                    <option value="data_service">Hyper DataService</option>
                  </AppSelect>
                </label>
                <label v-if="stage.execution === 'grpc'" class="md:col-span-2">
                  <span class="auth-label">gRPC Endpoint</span>
                  <AppInput v-model="stage.endpoint" placeholder="http://function-service:50051" />
                </label>
                <template v-else-if="stage.dataservice">
                  <label><span class="auth-label">DataService URL</span><AppInput v-model="stage.dataservice.base_url" /></label>
                  <label><span class="auth-label">Datasource</span><AppInput v-model="stage.dataservice.datasource" /></label>
                  <label><span class="auth-label">Named Mapper</span><AppInput v-model="stage.dataservice.mapper" /></label>
                  <label><span class="auth-label">Lookup Field</span><AppInput v-model="stage.dataservice.lookup_field" /></label>
                  <label><span class="auth-label">Result Key Field</span><AppInput v-model="stage.dataservice.result_key_field" /></label>
                  <label><span class="auth-label">Output Field</span><AppInput v-model="stage.dataservice.output_field" /></label>
                </template>
                <label>
                  <span class="auth-label">Failure Policy</span>
                  <AppSelect v-model="stage.failure_policy">
                    <option value="dead_letter">Dead Letter</option>
                    <option value="fail_pipeline">Fail Pipeline</option>
                    <option value="emit_unchanged">Emit Unchanged</option>
                    <option value="drop">Drop</option>
                  </AppSelect>
                </label>
                <label><span class="auth-label">Timeout (ms)</span><AppInput v-model="stage.timeout_ms" type="number" /></label>
                <label><span class="auth-label">Maximum Attempts</span><AppInput v-model="stage.retry.max_attempts" type="number" /></label>
              </div>
            </article>
          </div>
        </section>

        <section class="page-toolbar">
          <div class="flex items-center gap-1.5">
            <p class="text-sm font-medium">Configuration only</p>
            <AppInfoHint text="Run the separate cdc-processor service with this pipeline contract." />
          </div>
          <div class="flex gap-2">
            <AppButton v-if="selectedId" variant="destructive" @click="removePipeline">
              <Trash2 class="h-4 w-4" />Delete
            </AppButton>
            <AppButton variant="primary" :disabled="saving" @click="save">
              <Save class="h-4 w-4" />{{ saving ? "Saving…" : "Validate & Save" }}
            </AppButton>
          </div>
        </section>
      </div>
    </section>
  </div>
</template>
