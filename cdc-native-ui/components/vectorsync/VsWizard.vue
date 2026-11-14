<script setup lang="ts">
// Create / edit a vector sync in five steps. Every step can prove itself before moving on:
// tables show how many records their topics still hold, the model and the store have a Test
// button that runs inside the sidecar, and the Documents step previews real records live.
import {
  AlertTriangle, ArrowLeft, ArrowRight, Check, CheckCircle2, Database, Eye, FlaskConical, Loader2,
  RefreshCw, Rocket, Search, ServerCog, XCircle,
} from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import type { VsCapability, VsCheck, VsPreviewTopic, VsProfile, VsSources } from "~/composables/useVectorSync"
import {
  DEMO_TARGET, PROVIDERS, STEPS, SUGGESTED_MODELS, emptyVsDraft, emptyTable, fmtCount, fromProfile, projectionOf,
  idStrategyOf, rebuildReasons, sensitiveColumns, slug, stepProblems, tableOf, vsToRequest, type Step, type VsDraft,
} from "~/utils/vectorSync"

const props = defineProps<{ profile?: VsProfile | null; capability: VsCapability | null }>()
const emit = defineEmits<{ saved: [p: VsProfile]; cancel: [] }>()

const api = useVectorSync()
const toast = useToast()
const editing = computed(() => !!props.profile)
const original = props.profile ? fromProfile(props.profile) : null
const draft = reactive<VsDraft>(props.profile ? fromProfile(props.profile) : emptyVsDraft())

// ── steps ───────────────────────────────────────────────────────────────────────────────
const stepIdx = ref(0)
const step = computed<Step>(() => STEPS[stepIdx.value].key)
const visited = ref(new Set<number>([0]))
const showProblems = ref(false)
const columns = reactive<Record<string, string[]>>({})
const docling = computed(() => !!props.capability?.docling?.available)
const problems = computed(() => stepProblems(draft, step.value, columns, docling.value))
const go = (i: number) => {
  if (i > stepIdx.value) {
    for (let s = stepIdx.value; s < i; s++) {
      if (stepProblems(draft, STEPS[s].key, columns, docling.value).length) {
        stepIdx.value = s
        showProblems.value = true
        return
      }
    }
  }
  showProblems.value = false
  stepIdx.value = i
  visited.value.add(i)
}
const next = () => (problems.value.length ? (showProblems.value = true) : go(stepIdx.value + 1))

// ── step 1: tables ──────────────────────────────────────────────────────────────────────
const sources = ref<VsSources | null>(null)
const sourcesError = ref("")
const pipelineId = ref<string | "other" | null>(draft.source_profile_id)
const topicFilter = ref("")
const loadSources = async () => {
  try {
    sources.value = await api.sources()
    sourcesError.value = ""
    if (!pipelineId.value) {
      const first = sources.value.pipelines.find((p) => p.topics.length)
      pipelineId.value = first?.profile_id ?? (sources.value.other.length ? "other" : null)
    }
  } catch (e: any) {
    sourcesError.value = formatApiError(e, "Could not list topics")
  }
}
const pipeline = computed(() => sources.value?.pipelines.find((p) => p.profile_id === pipelineId.value) || null)
const topicsShown = computed(() => {
  const list = pipelineId.value === "other" ? sources.value?.other || [] : pipeline.value?.topics || []
  const f = topicFilter.value.trim().toLowerCase()
  return f ? list.filter((t) => t.name.toLowerCase().includes(f)) : list
})
const allShownSelected = computed(() => topicsShown.value.length > 0 && topicsShown.value.every((t) => draft.topics.includes(t.name)))
const toggleTopic = (name: string) => {
  draft.topics = draft.topics.includes(name) ? draft.topics.filter((t) => t !== name) : [...draft.topics, name].sort()
}
const toggleAllShown = () => {
  const names = topicsShown.value.map((t) => t.name)
  draft.topics = allShownSelected.value ? draft.topics.filter((t) => !names.includes(t)) : [...new Set([...draft.topics, ...names])].sort()
}
const selectPipeline = (id: string | "other") => {
  if (id !== pipelineId.value && draft.topics.length && !editing.value) draft.topics = []
  pipelineId.value = id
  draft.source_profile_id = id === "other" ? null : id
}
const trimmedSelected = computed(() => {
  const all = [...(sources.value?.pipelines.flatMap((p) => p.topics) || []), ...(sources.value?.other || [])]
  return all.filter((t) => draft.topics.includes(t.name) && t.trimmed).map((t) => tableOf(t.name))
})
const pipelineNameOf = (id: string | null) => sources.value?.pipelines.find((p) => p.profile_id === id)?.name
watch(
  () => draft.topics.map(tableOf),
  (tables) => {
    for (const t of tables) if (!draft.tables[t]) draft.tables[t] = emptyTable(columns[t] || [])
  },
  { immediate: true },
)

// ── step 2: model ───────────────────────────────────────────────────────────────────────
const provider = computed(() => PROVIDERS.find((p) => p.key === draft.embedding.provider)!)
const models = ref<string[]>([])
const modelsNote = ref("")
const modelsLoading = ref(false)
const modelCheck = ref<VsCheck | null>(null)
const modelTesting = ref(false)
const setProvider = (key: (typeof PROVIDERS)[number]["key"]) => {
  if (key === draft.embedding.provider) return
  const p = PROVIDERS.find((x) => x.key === key)!
  draft.embedding.provider = key
  draft.embedding.base_url = p.defaultUrl
  draft.embedding.model = p.defaultModel
  draft.embedding.api_key = ""
  modelCheck.value = null
  void loadModels()
}
const loadModels = async () => {
  if (draft.embedding.provider === "hash") {
    models.value = ["hash-256"]
    modelsNote.value = ""
    return
  }
  modelsLoading.value = true
  try {
    const r = await api.models(vsToRequest(draft).embedding, props.profile?.id)
    models.value = r.models
    modelsNote.value = r.error ? `Could not list models: ${r.error}` : `${r.models.length} model${r.models.length === 1 ? "" : "s"} on this server`
  } catch (e: any) {
    modelsNote.value = formatApiError(e, "Could not list models")
  } finally {
    modelsLoading.value = false
  }
}
const modelMissing = computed(() => draft.embedding.provider === "ollama" && models.value.length > 0 && !models.value.some((m) => m === draft.embedding.model || m === `${draft.embedding.model}:latest`))
const testModel = async () => {
  modelTesting.value = true
  try {
    const r = await api.test({ embedding: vsToRequest(draft).embedding, profile_id: props.profile?.id })
    modelCheck.value = r.checks[0] || null
  } catch (e: any) {
    modelCheck.value = { name: "embedding", ok: false, detail: formatApiError(e, "Test failed") }
  } finally {
    modelTesting.value = false
  }
}

// ── step 3: store ───────────────────────────────────────────────────────────────────────
const storeChecks = ref<VsCheck[] | null>(null)
const storeTesting = ref(false)
const collectionTouched = ref(editing.value)
const suggestedCollection = computed(() => slug(draft.name || (pipeline.value ? `${pipeline.value.name}_docs` : draft.topics.length === 1 ? `${tableOf(draft.topics[0])}_docs` : "cdc_docs")))
watch(suggestedCollection, (v) => {
  if (!collectionTouched.value) draft.target.collection = v
}, { immediate: true })
const useDemo = () => {
  Object.assign(draft.target, DEMO_TARGET)
  storeChecks.value = null
}
const testStore = async () => {
  storeTesting.value = true
  try {
    const r = await api.test({ target: vsToRequest(draft).target, profile_id: props.profile?.id })
    storeChecks.value = r.checks
  } catch (e: any) {
    storeChecks.value = [{ name: "postgres", ok: false, detail: formatApiError(e, "Test failed") }]
  } finally {
    storeTesting.value = false
  }
}

// ── step 4: documents (live preview) ────────────────────────────────────────────────────
const activeTable = ref("")
const preview = ref<Record<string, VsPreviewTopic>>({})
const previewLoading = ref(false)
const previewError = ref("")
const pkText = ref(draft.pk_fields.join(", "))
watch(pkText, (v) => (draft.pk_fields = v.split(",").map((s) => s.trim()).filter(Boolean)))
const tablesSelected = computed(() => draft.topics.map(tableOf))
watch(tablesSelected, (t) => {
  if (!t.includes(activeTable.value)) activeTable.value = t[0] || ""
}, { immediate: true })

const runPreview = async () => {
  if (!draft.topics.length) return
  previewLoading.value = true
  try {
    const r = await api.preview({ topics: draft.topics, projection: projectionOf(draft), id_strategy: idStrategyOf(draft), delete_mode: draft.delete_mode, sample: 3 })
    const map: Record<string, VsPreviewTopic> = {}
    for (const t of r.topics) {
      map[tableOf(t.topic)] = t
      if (t.columns.length) {
        const name = tableOf(t.topic)
        const firstTime = !columns[name]?.length
        columns[name] = t.columns
        // Columns arrived for the first time: start credentials out excluded.
        const td = draft.tables[name]
        if (firstTime && td && !td.exclude_fields.length && td.mode === "all") {
          const s = sensitiveColumns(t.columns)
          if (s.length) {
            draft.tables[name] = { ...td, exclude_fields: s }
            return void runPreview()
          }
        }
      }
    }
    preview.value = map
    previewError.value = ""
  } catch (e: any) {
    previewError.value = formatApiError(e, "Preview failed")
  } finally {
    previewLoading.value = false
  }
}
let previewTimer: ReturnType<typeof setTimeout> | null = null
watch(
  () => [step.value, JSON.stringify(projectionOf(draft)), draft.pk_fields.join(","), draft.delete_mode],
  () => {
    if (step.value !== "documents") return
    if (previewTimer) clearTimeout(previewTimer)
    previewTimer = setTimeout(runPreview, 450)
  },
)
const activePreview = computed(() => preview.value[activeTable.value])
const avgChars = computed(() => {
  const r = activePreview.value?.records.filter((x) => x.chars) || []
  return r.length ? Math.round(r.reduce((a, x) => a + (x.chars || 0), 0) / r.length) : null
})

// ── step 5: review + save ───────────────────────────────────────────────────────────────
const nameTouched = ref(editing.value)
watch(
  () => [pipeline.value?.name, draft.topics.length],
  () => {
    if (nameTouched.value) return
    const p = pipeline.value?.name
    draft.name = p ? `${p} search` : draft.topics.length === 1 ? `${tableOf(draft.topics[0])} search` : draft.topics.length ? "CDC search" : ""
  },
  { immediate: true },
)
const rebuild = computed(() => rebuildReasons(original, draft))
const saving = ref(false)
const save = async () => {
  for (let i = 0; i < STEPS.length; i++) {
    if (stepProblems(draft, STEPS[i].key, columns, docling.value).length) {
      stepIdx.value = i
      showProblems.value = true
      return
    }
  }
  saving.value = true
  try {
    const body = vsToRequest(draft)
    const p = editing.value ? await api.update(props.profile!.id, body) : await api.create(body)
    toast.success(editing.value ? (rebuild.value.length ? "Saved — rebuilding the collection" : "Saved") : draft.enabled ? "Vector sync started" : "Vector sync saved (paused)")
    emit("saved", p)
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not save"))
  } finally {
    saving.value = false
  }
}

onMounted(() => {
  void loadSources()
  void loadModels()
})
onBeforeUnmount(() => previewTimer && clearTimeout(previewTimer))

const SSL = ["", "disable", "prefer", "require", "verify-ca", "verify-full"]
</script>

<template>
  <div class="space-y-4">
    <!-- stepper -->
    <nav class="stat-card p-2" aria-label="Steps">
      <ol class="flex flex-wrap items-center gap-1">
        <li v-for="(s, i) in STEPS" :key="s.key" class="flex items-center gap-1">
          <button
            type="button"
            class="inline-flex h-9 items-center gap-2 rounded-lg px-3 text-sm transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            :class="i === stepIdx ? 'bg-brand-500/10 font-medium text-primary' : visited.has(i) ? 'text-foreground hover:bg-muted' : 'text-muted-foreground hover:bg-muted'"
            :aria-current="i === stepIdx ? 'step' : undefined"
            @click="go(i)"
          >
            <span
              class="flex h-5 w-5 items-center justify-center rounded-full text-[11px] font-semibold"
              :class="i < stepIdx || (visited.has(i) && i !== stepIdx && !stepProblems(draft, s.key, columns, docling).length) ? 'bg-primary text-primary-foreground' : i === stepIdx ? 'border border-primary text-primary' : 'border border-border'"
            >
              <Check v-if="i < stepIdx" class="h-3 w-3" :stroke-width="2.5" />
              <template v-else>{{ i + 1 }}</template>
            </span>
            {{ s.label }}
          </button>
          <span v-if="i < STEPS.length - 1" class="hidden h-px w-4 bg-border sm:block" />
        </li>
      </ol>
    </nav>

    <div v-if="showProblems && problems.length" class="alert-error text-sm">
      <p v-for="p in problems" :key="p">{{ p }}</p>
    </div>

    <!-- 1. tables -->
    <section v-if="step === 'source'" class="space-y-4">
      <div class="stat-card">
        <AppSectionTitle title="Which tables should AI be able to search?" />
        <p class="mt-1 text-sm text-muted-foreground">The sync reads the change topics your pipelines already produce — no extra load on the source database.</p>
        <p v-if="sourcesError" class="alert-error mt-3 text-sm">{{ sourcesError }}</p>
        <div v-else-if="!sources" class="mt-4 flex items-center gap-2 text-sm text-muted-foreground"><Loader2 class="h-4 w-4 animate-spin" :stroke-width="1.75" />Reading topics…</div>
        <div v-else class="mt-4 grid gap-4 lg:grid-cols-[280px_1fr]">
          <div class="space-y-1">
            <p class="px-1 text-xs font-medium text-muted-foreground">Pipeline</p>
            <button
              v-for="p in sources.pipelines"
              :key="p.profile_id"
              type="button"
              class="flex w-full items-center justify-between gap-2 rounded-lg px-3 py-2 text-left text-sm transition-colors"
              :class="pipelineId === p.profile_id ? 'bg-brand-500/10 text-primary' : 'hover:bg-muted'"
              @click="selectPipeline(p.profile_id)"
            >
              <span class="flex min-w-0 items-center gap-2"><AppDbIcon :type="p.source_type" size="sm" class="shrink-0" /><span class="truncate">{{ p.name }}</span></span>
              <span class="shrink-0 text-xs tabular-nums text-muted-foreground">{{ p.topics.length }}</span>
            </button>
            <button
              v-if="sources.other.length"
              type="button"
              class="flex w-full items-center justify-between gap-2 rounded-lg px-3 py-2 text-left text-sm transition-colors"
              :class="pipelineId === 'other' ? 'bg-brand-500/10 text-primary' : 'hover:bg-muted'"
              @click="selectPipeline('other')"
            >
              <span class="text-muted-foreground">Other topics</span>
              <span class="text-xs tabular-nums text-muted-foreground">{{ sources.other.length }}</span>
            </button>
            <p v-if="!sources.pipelines.length && !sources.other.length" class="px-1 text-sm text-muted-foreground">No pipelines yet. Create and start a CDC pipeline first.</p>
          </div>

          <div class="min-w-0">
            <div class="mb-2 flex items-center gap-2">
              <div class="relative flex-1">
                <Search class="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
                <input v-model="topicFilter" class="auth-input pl-8" placeholder="Filter tables" />
              </div>
              <button type="button" class="btn-secondary w-auto" :disabled="!topicsShown.length" @click="toggleAllShown">{{ allShownSelected ? "Clear" : "Select all" }}</button>
              <button type="button" class="icon-button" title="Reload topics" @click="loadSources"><RefreshCw class="h-4 w-4" :stroke-width="1.75" /></button>
            </div>
            <div class="max-h-[22rem] overflow-y-auto rounded-lg border border-border">
              <label
                v-for="t in topicsShown"
                :key="t.name"
                class="flex cursor-pointer items-center gap-3 border-b border-border px-3 py-2 last:border-b-0 hover:bg-muted/50"
              >
                <input type="checkbox" class="h-4 w-4 rounded border-border accent-brand-500" :checked="draft.topics.includes(t.name)" @change="toggleTopic(t.name)" />
                <div class="min-w-0 flex-1">
                  <p class="truncate text-sm font-medium text-foreground">{{ t.table || tableOf(t.name) }}</p>
                  <p class="truncate font-mono text-[11px] text-muted-foreground">{{ t.name }}</p>
                </div>
                <span v-if="t.trimmed" class="badge-warning shrink-0" title="Topic retention already deleted its oldest records">partial history</span>
                <span class="w-20 shrink-0 text-right text-xs tabular-nums text-muted-foreground">{{ fmtCount(t.messages) }} rec.</span>
              </label>
              <p v-if="!topicsShown.length" class="px-3 py-6 text-center text-sm text-muted-foreground">
                {{ pipelineId && pipelineId !== "other" ? "This pipeline has no topics yet — start it so its snapshot creates them." : "No topics match." }}
              </p>
            </div>
            <p class="mt-2 text-xs text-muted-foreground">{{ draft.topics.length }} selected</p>
          </div>
        </div>
      </div>

      <div class="stat-card">
        <AppSectionTitle title="Where to start" />
        <div class="mt-3 grid gap-2 sm:grid-cols-2">
          <button
            v-for="o in [
              { key: 'earliest', title: 'Backfill, then follow changes', text: 'Embed every record the topics still hold, then keep up with new changes.' },
              { key: 'latest', title: 'New changes only', text: 'Skip history. Only rows that change from now on are embedded.' },
            ]"
            :key="o.key"
            type="button"
            class="rounded-lg border px-3 py-2.5 text-left transition-colors"
            :class="draft.start_from === o.key ? 'border-primary bg-brand-500/10' : 'border-border hover:bg-muted'"
            @click="draft.start_from = o.key as any"
          >
            <p class="text-sm font-medium" :class="draft.start_from === o.key ? 'text-primary' : 'text-foreground'">{{ o.title }}</p>
            <p class="mt-0.5 text-xs text-muted-foreground">{{ o.text }}</p>
          </button>
        </div>
        <p v-if="draft.start_from === 'earliest' && trimmedSelected.length" class="alert-warning mt-3 text-xs">
          <AlertTriangle class="mr-1 inline h-3.5 w-3.5" :stroke-width="1.75" />
          {{ trimmedSelected.join(", ") }}: retention already deleted older records, so the backfill covers only what is left. For a complete copy, run
          <b>Resync</b> on the pipeline (it re-snapshots the tables into the topics) — the sync picks the rows up automatically.
        </p>
      </div>
    </section>

    <!-- 2. model -->
    <section v-if="step === 'model'" class="space-y-4">
      <div class="stat-card">
        <AppSectionTitle title="Embedding model" />
        <p class="mt-1 text-sm text-muted-foreground">Turns each row's text into a vector. The same model must be used to search, so changing it later rebuilds the collection.</p>
        <div class="mt-4 grid gap-2 sm:grid-cols-3">
          <button
            v-for="p in PROVIDERS"
            :key="p.key"
            type="button"
            class="rounded-lg border px-3 py-2.5 text-left transition-colors"
            :class="draft.embedding.provider === p.key ? 'border-primary bg-brand-500/10' : 'border-border hover:bg-muted'"
            @click="setProvider(p.key)"
          >
            <p class="text-sm font-medium" :class="draft.embedding.provider === p.key ? 'text-primary' : 'text-foreground'">{{ p.label }}</p>
            <p class="mt-0.5 text-xs text-muted-foreground">{{ p.hint }}</p>
          </button>
        </div>

        <div class="mt-5 grid gap-4 md:grid-cols-2">
          <label v-if="draft.embedding.provider !== 'hash'" class="block">
            <AppFormLabel label="Server URL" :hint="draft.embedding.provider === 'ollama' ? 'From the sidecar container: host.docker.internal is this machine.' : 'Base URL that serves /embeddings.'" />
            <input v-model="draft.embedding.base_url" class="auth-input font-mono" :placeholder="provider.defaultUrl" @change="loadModels" />
          </label>
          <label v-if="draft.embedding.provider === 'openai'" class="block">
            <AppFormLabel label="API key" hint="Write-only. Stored encrypted when ORCH_SECRET_KEY is set." />
            <input v-model="draft.embedding.api_key" type="password" autocomplete="new-password" class="auth-input" placeholder="sk-…" @change="loadModels" />
          </label>
          <label class="block">
            <AppFormLabel label="Model" />
            <div class="flex gap-2">
              <input v-model="draft.embedding.model" list="vs-models" class="auth-input font-mono" :placeholder="provider.defaultModel" />
              <button type="button" class="icon-button h-9 w-9 shrink-0" title="Load models from the server" :disabled="modelsLoading" @click="loadModels">
                <component :is="modelsLoading ? Loader2 : RefreshCw" class="h-4 w-4" :class="modelsLoading && 'animate-spin'" :stroke-width="1.75" />
              </button>
            </div>
            <datalist id="vs-models"><option v-for="m in models" :key="m" :value="m" /></datalist>
            <p v-if="modelsNote" class="mt-1 text-xs text-muted-foreground">{{ modelsNote }}</p>
            <p v-if="modelMissing" class="mt-1 text-xs text-amber-700 dark:text-amber-400">Not on this Ollama server yet — run <span class="font-mono">ollama pull {{ draft.embedding.model }}</span>.</p>
          </label>
        </div>

        <div class="mt-3 flex flex-wrap gap-1.5">
          <button
            v-for="m in SUGGESTED_MODELS[draft.embedding.provider]"
            :key="m.name"
            type="button"
            class="rounded-md border px-2 py-1 text-left text-xs transition-colors"
            :class="draft.embedding.model === m.name ? 'border-primary/40 bg-brand-500/10 text-primary' : 'border-border text-muted-foreground hover:bg-muted'"
            @click="draft.embedding.model = m.name"
          >
            <span class="font-mono">{{ m.name }}</span> · {{ m.note }}
          </button>
        </div>

        <details class="mt-4">
          <summary class="cursor-pointer text-xs font-medium text-muted-foreground">Advanced</summary>
          <div class="mt-3 grid gap-4 md:grid-cols-2">
            <label class="block">
              <AppFormLabel label="Dimensions" hint="Leave empty for the model's default. Only models that support truncation accept another value." />
              <input :value="draft.embedding.dimensions ?? ''" type="number" min="1" class="auth-input" placeholder="model default" @input="draft.embedding.dimensions = ($event.target as HTMLInputElement).value ? Number(($event.target as HTMLInputElement).value) : null" />
            </label>
            <label class="block">
              <AppFormLabel label="Texts per request" hint="Default 32." />
              <input :value="draft.embedding.batch_size ?? ''" type="number" min="1" max="512" class="auth-input" placeholder="32" @input="draft.embedding.batch_size = ($event.target as HTMLInputElement).value ? Number(($event.target as HTMLInputElement).value) : null" />
            </label>
          </div>
        </details>

        <div class="mt-5 flex flex-wrap items-center gap-3 border-t border-border pt-4">
          <button type="button" class="btn-secondary w-auto" :disabled="modelTesting" @click="testModel">
            <component :is="modelTesting ? Loader2 : FlaskConical" class="h-4 w-4" :class="modelTesting && 'animate-spin'" :stroke-width="1.75" />Test model
          </button>
          <div v-if="modelCheck" class="flex min-w-0 items-start gap-2 text-sm">
            <component :is="modelCheck.ok ? CheckCircle2 : XCircle" class="mt-0.5 h-4 w-4 shrink-0" :class="modelCheck.ok ? 'text-emerald-600' : 'text-destructive'" :stroke-width="1.75" />
            <div class="min-w-0">
              <p :class="modelCheck.ok ? 'text-foreground' : 'text-destructive'">{{ modelCheck.detail }}</p>
              <p v-if="modelCheck.hint" class="text-xs text-muted-foreground">{{ modelCheck.hint }}</p>
            </div>
          </div>
        </div>
        <p v-if="rebuild.length && rebuild.some((r) => r.includes('model'))" class="alert-warning mt-3 text-xs">Changing the model rebuilds the collection: existing vectors are deleted and the topics are read again.</p>
      </div>
    </section>

    <!-- 3. store -->
    <section v-if="step === 'store'" class="space-y-4">
      <div class="stat-card">
        <div class="flex flex-wrap items-start justify-between gap-3">
          <div>
            <AppSectionTitle title="Vector store (PostgreSQL + pgvector)" />
            <p class="mt-1 text-sm text-muted-foreground">Vectors land in LangChain's PGVector tables, one collection per sync — ready for LangChain, LangGraph or plain SQL.</p>
          </div>
          <button type="button" class="btn-secondary w-auto" @click="useDemo"><Database class="h-4 w-4" :stroke-width="1.75" />Use demo store</button>
        </div>
        <div class="mt-4 grid gap-4 md:grid-cols-6">
          <label class="block md:col-span-4"><AppFormLabel label="Host" hint="As the sidecar container sees it." /><input v-model="draft.target.host" class="auth-input" placeholder="pgvector.internal" /></label>
          <label class="block md:col-span-2"><AppFormLabel label="Port" /><input v-model="draft.target.port" class="auth-input" placeholder="5432" /></label>
          <label class="block md:col-span-2"><AppFormLabel label="Database" /><input v-model="draft.target.database" class="auth-input" /></label>
          <label class="block md:col-span-2"><AppFormLabel label="Username" /><input v-model="draft.target.username" class="auth-input" autocomplete="off" /></label>
          <label class="block md:col-span-2"><AppFormLabel label="Password" /><input v-model="draft.target.password" type="password" autocomplete="new-password" class="auth-input" /></label>
          <label class="block md:col-span-2">
            <AppFormLabel label="TLS (sslmode)" />
            <select v-model="draft.target.sslmode" class="auth-input"><option v-for="m in SSL" :key="m" :value="m">{{ m || "server default" }}</option></select>
          </label>
          <label class="block md:col-span-4">
            <AppFormLabel label="Collection" hint="Name of this sync's collection in langchain_pg_collection. Search with the same name." />
            <input v-model="draft.target.collection" class="auth-input font-mono" @input="collectionTouched = true" />
          </label>
        </div>
        <div class="mt-5 flex flex-wrap items-start gap-3 border-t border-border pt-4">
          <button type="button" class="btn-secondary w-auto" :disabled="storeTesting" @click="testStore">
            <component :is="storeTesting ? Loader2 : ServerCog" class="h-4 w-4" :class="storeTesting && 'animate-spin'" :stroke-width="1.75" />Test connection
          </button>
          <ul v-if="storeChecks" class="min-w-0 flex-1 space-y-1.5">
            <li v-for="c in storeChecks" :key="c.name" class="flex items-start gap-2 text-sm">
              <component :is="c.ok ? CheckCircle2 : XCircle" class="mt-0.5 h-4 w-4 shrink-0" :class="c.ok ? 'text-emerald-600' : 'text-destructive'" :stroke-width="1.75" />
              <div class="min-w-0"><p :class="c.ok ? 'text-foreground' : 'text-destructive'">{{ c.detail }}</p><p v-if="c.hint" class="text-xs text-muted-foreground">{{ c.hint }}</p></div>
            </li>
          </ul>
        </div>
        <p v-if="rebuild.some((r) => r.includes('collection'))" class="alert-warning mt-3 text-xs">A new collection is filled from the topics again; the old one stays in Postgres.</p>
      </div>
    </section>

    <!-- 4. documents -->
    <section v-if="step === 'documents'" class="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <div class="stat-card p-0">
        <div class="flex flex-wrap gap-x-1 border-b border-border px-2" role="tablist">
          <button
            v-for="t in tablesSelected"
            :key="t"
            type="button"
            role="tab"
            :aria-selected="activeTable === t"
            class="-mb-px inline-flex h-11 items-center gap-2 border-b-2 px-3 text-sm font-medium transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            :class="activeTable === t ? 'border-primary text-primary' : 'border-transparent text-muted-foreground hover:text-foreground'"
            @click="activeTable = t"
          >
            {{ t }}
            <span v-if="stepProblems({ ...draft, topics: draft.topics.filter((x) => tableOf(x) === t) }, 'documents', columns, docling).length" class="h-1.5 w-1.5 rounded-full bg-destructive" />
          </button>
        </div>
        <div class="p-4">
          <VsTableDocs v-if="activeTable && draft.tables[activeTable]" v-model="draft.tables[activeTable]" :table="activeTable" :columns="columns[activeTable] || []" :docling="capability?.docling || null" />
          <details class="mt-6 border-t border-border pt-4">
            <summary class="cursor-pointer text-xs font-medium text-muted-foreground">Deletes and document ids (all tables)</summary>
            <div class="mt-3 grid gap-4 md:grid-cols-2">
              <div>
                <AppFormLabel label="When a row is deleted" />
                <div class="mt-1 flex gap-2">
                  <button type="button" class="btn-secondary w-auto flex-1" :class="draft.delete_mode === 'hard' && 'border-primary text-primary'" @click="draft.delete_mode = 'hard'">Remove vector</button>
                  <button type="button" class="btn-secondary w-auto flex-1" :class="draft.delete_mode === 'soft' && 'border-primary text-primary'" @click="draft.delete_mode = 'soft'">Keep, mark deleted</button>
                </div>
                <p class="mt-1 text-xs text-muted-foreground">“Keep” stores <span class="font-mono">_is_deleted=true</span>; filter it out when searching. Needs the full old row (REPLICA IDENTITY FULL on Postgres).</p>
              </div>
              <label class="block">
                <AppFormLabel label="Key columns (optional)" hint="Only for topics without a Kafka key (e.g. incremental JDBC capture). Comma-separated." />
                <input v-model="pkText" class="auth-input font-mono" placeholder="id" />
              </label>
            </div>
          </details>
        </div>
      </div>

      <div class="stat-card">
        <div class="flex items-center justify-between gap-2">
          <div class="flex items-center gap-2">
            <Eye class="h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
            <p class="text-sm font-semibold text-foreground">Live preview</p>
            <span v-if="avgChars" class="text-xs text-muted-foreground">~{{ avgChars }} characters per document</span>
          </div>
          <button type="button" class="icon-button" title="Read the newest records again" :disabled="previewLoading" @click="runPreview">
            <component :is="previewLoading ? Loader2 : RefreshCw" class="h-4 w-4" :class="previewLoading && 'animate-spin'" :stroke-width="1.75" />
          </button>
        </div>
        <p class="mt-1 text-xs text-muted-foreground">The newest records of <span class="font-mono">{{ activeTable }}</span>, exactly as they will be embedded. Nothing is written.</p>
        <p v-if="previewError" class="alert-error mt-3 text-sm">{{ previewError }}</p>
        <p v-else-if="activePreview?.error" class="alert-info mt-3 text-sm">{{ activePreview.error }}</p>
        <div v-else-if="!activePreview" class="mt-6 flex items-center gap-2 text-sm text-muted-foreground"><Loader2 class="h-4 w-4 animate-spin" :stroke-width="1.75" />Reading records…</div>
        <div v-else class="mt-3 space-y-3">
          <article v-for="r in activePreview.records" :key="`${r.partition}-${r.offset}`" class="rounded-lg border border-border">
            <header class="flex items-center justify-between gap-2 border-b border-border px-3 py-1.5">
              <span class="truncate font-mono text-[11px] text-muted-foreground" :title="r.doc_id">{{ r.doc_id || `offset ${r.offset}` }}</span>
              <span v-if="r.error" class="badge-error shrink-0">would dead-letter</span>
              <span v-else-if="r.action === 'delete'" class="badge-warning shrink-0">delete</span>
              <span v-else-if="r.skipped" class="badge-neutral shrink-0">skipped</span>
              <span v-else class="badge-success shrink-0">{{ r.op === "r" ? "snapshot" : r.op === "c" ? "insert" : r.op === "u" ? "update" : r.op === "d" ? "soft delete" : "upsert" }}</span>
            </header>
            <p v-if="r.error" class="px-3 py-2 text-xs text-destructive">{{ r.error }}</p>
            <p v-else-if="r.skipped" class="px-3 py-2 text-xs text-muted-foreground">{{ r.skipped }}</p>
            <p v-else-if="r.action === 'delete'" class="px-3 py-2 text-xs text-muted-foreground">The vector with this id is removed.</p>
            <template v-else>
              <div v-if="r.chunks" class="flex items-center gap-2 border-b border-border px-3 py-1.5 text-[11px] text-muted-foreground">
                <span class="badge-info">{{ r.chunks }} chunk{{ r.chunks === 1 ? "" : "s" }}</span>
                <span v-if="r.metadata?._doc_format">{{ String(r.metadata._doc_format).toUpperCase() }}<template v-if="r.metadata?._doc_pages"> · {{ r.metadata._doc_pages }} page(s)</template></span>
                <span>→ {{ r.chunks }} vector{{ r.chunks === 1 ? "" : "s" }} for this row</span>
              </div>
              <template v-if="r.chunk_samples?.length">
                <pre v-for="(c, ci) in r.chunk_samples" :key="ci" class="max-h-32 overflow-auto whitespace-pre-wrap break-words border-b border-dashed border-border px-3 py-2 font-sans text-xs text-foreground last:border-b-0"><span class="mr-1 font-mono text-[10px] text-muted-foreground">#{{ ci }}</span>{{ c }}</pre>
                <p v-if="(r.chunks || 0) > r.chunk_samples.length" class="px-3 py-1 text-[11px] text-muted-foreground">… {{ (r.chunks || 0) - r.chunk_samples.length }} more</p>
              </template>
              <pre v-else class="max-h-40 overflow-auto whitespace-pre-wrap break-words px-3 py-2 font-sans text-xs text-foreground">{{ r.content }}</pre>
              <details class="border-t border-border px-3 py-1.5">
                <summary class="cursor-pointer text-[11px] text-muted-foreground">metadata · {{ Object.keys(r.metadata || {}).length }} fields</summary>
                <pre class="mt-1 max-h-40 overflow-auto font-mono text-[11px] text-muted-foreground">{{ JSON.stringify(r.metadata, null, 2) }}</pre>
              </details>
            </template>
          </article>
          <p v-if="!activePreview.records.length" class="text-sm text-muted-foreground">No records to show.</p>
        </div>
      </div>
    </section>

    <!-- 5. review -->
    <section v-if="step === 'review'" class="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <div class="stat-card space-y-4">
        <AppSectionTitle title="Name it" />
        <label class="block"><AppFormLabel label="Name" /><input v-model="draft.name" class="auth-input" @input="nameTouched = true" /></label>
        <label class="block"><AppFormLabel label="Description (optional)" hint="What agents should use this collection for." /><textarea v-model="draft.description" rows="3" class="auth-input" placeholder="Customer support: find orders and their notes" /></label>
        <label v-if="!editing" class="flex items-start gap-2 text-sm">
          <input v-model="draft.enabled" type="checkbox" class="mt-0.5 h-4 w-4 rounded border-border accent-brand-500" />
          <span><span class="font-medium text-foreground">Start syncing now</span><br /><span class="text-xs text-muted-foreground">Leave off to save it paused.</span></span>
        </label>
      </div>
      <div class="stat-card">
        <AppSectionTitle title="Summary" />
        <dl class="mt-3 grid grid-cols-[8rem_1fr] gap-x-3 gap-y-2 text-sm">
          <dt class="text-muted-foreground">Tables</dt>
          <dd class="text-foreground">{{ draft.topics.map(tableOf).join(", ") }}<span v-if="pipelineNameOf(draft.source_profile_id)" class="text-muted-foreground"> · {{ pipelineNameOf(draft.source_profile_id) }}</span></dd>
          <dt class="text-muted-foreground">Start</dt>
          <dd class="text-foreground">{{ draft.start_from === "earliest" ? "Backfill, then follow changes" : "New changes only" }}</dd>
          <dt class="text-muted-foreground">Model</dt>
          <dd class="text-foreground"><span class="font-mono">{{ draft.embedding.model }}</span> · {{ PROVIDERS.find((p) => p.key === draft.embedding.provider)?.label }}</dd>
          <dt class="text-muted-foreground">Store</dt>
          <dd class="truncate text-foreground"><span class="font-mono">{{ draft.target.collection }}</span> on {{ draft.target.host }}/{{ draft.target.database }}</dd>
          <dt class="text-muted-foreground">Deletes</dt>
          <dd class="text-foreground">{{ draft.delete_mode === "hard" ? "Remove the vector" : "Keep, marked deleted" }}</dd>
        </dl>
        <p v-if="rebuild.length" class="alert-warning mt-4 text-xs">Saving rebuilds the collection because {{ rebuild.join(" and ") }}.</p>
        <p v-if="!capability?.secrets_encrypted" class="mt-4 text-xs text-muted-foreground">Passwords are stored in the orchestrator's config store. Set ORCH_SECRET_KEY to keep them encrypted at rest.</p>
      </div>
    </section>

    <!-- footer -->
    <div class="flex items-center justify-between gap-2">
      <button type="button" class="inline-flex h-9 items-center gap-2 rounded-lg px-3 text-sm font-medium text-muted-foreground hover:bg-muted hover:text-foreground" @click="stepIdx === 0 ? emit('cancel') : go(stepIdx - 1)">
        <ArrowLeft class="h-4 w-4" :stroke-width="1.75" />{{ stepIdx === 0 ? "Cancel" : "Back" }}
      </button>
      <button v-if="stepIdx < STEPS.length - 1" type="button" class="btn-primary w-auto" @click="next">Next: {{ STEPS[stepIdx + 1].label }}<ArrowRight class="h-4 w-4" :stroke-width="1.75" /></button>
      <button v-else type="button" class="btn-primary w-auto" :disabled="saving" @click="save">
        <component :is="saving ? Loader2 : Rocket" class="h-4 w-4" :class="saving && 'animate-spin'" :stroke-width="1.75" />
        {{ editing ? "Save changes" : draft.enabled ? "Create and start" : "Create" }}
      </button>
    </div>
  </div>
</template>
