<script setup lang="ts">
// One vector sync: is it keeping up (overview), does retrieval work (search), what failed
// (dead letters), and how it is configured (settings).
import {
  Activity, AlertOctagon, AlertTriangle, Boxes, Clock, Gauge, Hammer, Inbox, Loader2, Pause, Pencil, Play,
  Search, Settings2, Sparkles, Trash2, Zap,
} from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import type { VsHit, VsProfile } from "~/composables/useVectorSync"
import { STATE_LABEL, ago, badgeStatus, fmtCount, tableOf } from "~/utils/vectorSync"

const props = defineProps<{ sync: VsProfile; pipelineName?: string }>()
const emit = defineEmits<{ edit: []; changed: [p?: VsProfile]; deleted: [] }>()

const api = useVectorSync()
const toast = useToast()
const st = computed(() => props.sync.status || { state: "unknown" })
const c = computed(() => st.value.counters || {})

type Tab = "overview" | "search" | "dead" | "settings"
const tab = ref<Tab>("overview")
const tabs = computed(() => [
  { key: "overview" as Tab, label: "Overview", icon: Activity },
  { key: "search" as Tab, label: "Search", icon: Search },
  { key: "dead" as Tab, label: "Dead letters", icon: Inbox, count: c.value.dead_letters_stored || 0 },
  { key: "settings" as Tab, label: "Settings", icon: Settings2 },
])

// ── actions ─────────────────────────────────────────────────────────────────────────────
const busy = ref("")
const act = async (what: "pause" | "resume" | "rebuild") => {
  busy.value = what
  try {
    const p = await api[what](props.sync.id)
    toast.success(what === "pause" ? "Paused — offsets are kept" : what === "resume" ? "Resumed" : "Rebuilding from the start of the topics")
    emit("changed", p)
  } catch (e: any) {
    toast.error(formatApiError(e, `Could not ${what}`))
  } finally {
    busy.value = ""
    confirmRebuild.value = false
  }
}
const confirmRebuild = ref(false)
const confirmDelete = ref(false)
const dropCollection = ref(false)
const remove = async () => {
  busy.value = "delete"
  try {
    await api.remove(props.sync.id, dropCollection.value)
    toast.success(dropCollection.value ? "Deleted with its collection" : "Deleted — the collection stays in Postgres")
    emit("deleted")
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not delete"))
  } finally {
    busy.value = ""
    confirmDelete.value = false
  }
}

// ── search ──────────────────────────────────────────────────────────────────────────────
const query = ref("")
const k = ref(5)
const tableFilter = ref("")
const hits = ref<VsHit[] | null>(null)
const took = ref<number | null>(null)
const searching = ref(false)
const searchError = ref("")
const runSearch = async () => {
  if (!query.value.trim()) return
  searching.value = true
  try {
    const r = await api.search(props.sync.id, { query: query.value.trim(), k: k.value, ...(tableFilter.value ? { filter: { _table: tableFilter.value } } : {}) })
    hits.value = r.hits
    took.value = r.took_ms
    searchError.value = ""
  } catch (e: any) {
    searchError.value = formatApiError(e, "Search failed")
  } finally {
    searching.value = false
  }
}
const userMeta = (m: Record<string, any>) => Object.entries(m).filter(([key]) => !key.startsWith("_")).slice(0, 8)
const expanded = ref(new Set<string>())

// ── dead letters ────────────────────────────────────────────────────────────────────────
const dead = ref<{ items: any[]; total: number } | null>(null)
const deadLoading = ref(false)
const loadDead = async () => {
  deadLoading.value = true
  try {
    dead.value = await api.deadLetters(props.sync.id)
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not load dead letters"))
  } finally {
    deadLoading.value = false
  }
}
const clearDead = async () => {
  try {
    const r = await api.clearDeadLetters(props.sync.id)
    toast.success(`Cleared ${r.cleared}`)
    await loadDead()
    emit("changed")
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not clear"))
  }
}
watch(tab, (t) => t === "dead" && loadDead())

const snippet = computed(() => {
  const e = props.sync.embedding
  const t = props.sync.target
  const emb =
    e.provider === "openai"
      ? `from langchain_openai import OpenAIEmbeddings\nemb = OpenAIEmbeddings(model="${e.model}", base_url="${e.base_url}")`
      : e.provider === "ollama"
        ? `from langchain_ollama import OllamaEmbeddings\nemb = OllamaEmbeddings(model="${e.model}", base_url="${e.base_url}")`
        : "# hash vectors are demo-only; search from this page instead"
  return `${emb}
from langchain_postgres import PGVector

store = PGVector(
    connection="postgresql+psycopg://USER:PASS@${t.host}:${t.port || 5432}/${t.database}",
    collection_name="${t.collection}",
    embeddings=emb,
    use_jsonb=True,
)
docs = store.similarity_search("your question", k=5, filter={"_table": "${tableOf(props.sync.topics[0] || "")}"})`
})
const running = computed(() => ["running", "retrying", "connecting", "starting"].includes(st.value.state))
const lagTone = computed(() => ((st.value.lag?.total ?? 0) > 1000 ? "warning" : "neutral"))
const topicLag = computed(() => Object.entries(st.value.lag?.topics || {}).sort((a, b) => b[1] - a[1]))
</script>

<template>
  <div class="space-y-4">
    <!-- header -->
    <section class="page-toolbar">
      <div class="min-w-0">
        <div class="flex items-center gap-2">
          <h1 class="page-title truncate">{{ sync.name }}</h1>
          <AppStatusBadge :status="badgeStatus(st.state)" :label="STATE_LABEL[st.state] || st.state" />
        </div>
        <p class="mt-0.5 max-w-3xl truncate text-sm text-muted-foreground">
          {{ sync.description || `${pipelineName || "Topics"} → ${sync.embedding.model} → ${sync.target.collection}` }}
        </p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <button v-if="sync.enabled" type="button" class="btn-secondary w-auto" :disabled="!!busy" @click="act('pause')"><Pause class="h-4 w-4" :stroke-width="1.75" />Pause</button>
        <button v-else type="button" class="btn-secondary w-auto" :disabled="!!busy" @click="act('resume')"><Play class="h-4 w-4" :stroke-width="1.75" />Resume</button>
        <button type="button" class="btn-secondary w-auto" :disabled="!!busy" @click="confirmRebuild = true"><Hammer class="h-4 w-4" :stroke-width="1.75" />Rebuild</button>
        <button type="button" class="btn-secondary w-auto" @click="emit('edit')"><Pencil class="h-4 w-4" :stroke-width="1.75" />Edit</button>
        <button type="button" class="icon-button" title="Delete" @click="confirmDelete = true"><Trash2 class="h-4 w-4" :stroke-width="1.75" /></button>
      </div>
    </section>

    <div class="stat-card p-0">
      <div class="flex flex-wrap gap-x-1 border-b border-border px-2" role="tablist">
        <button
          v-for="t in tabs"
          :key="t.key"
          type="button"
          role="tab"
          :aria-selected="tab === t.key"
          class="-mb-px inline-flex h-11 items-center gap-2 border-b-2 px-3 text-sm font-medium transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          :class="tab === t.key ? 'border-primary text-primary' : 'border-transparent text-muted-foreground hover:text-foreground'"
          @click="tab = t.key"
        >
          <component :is="t.icon" class="h-4 w-4" :stroke-width="1.75" />{{ t.label }}
          <span v-if="t.count" class="rounded-full bg-destructive/10 px-1.5 text-[11px] tabular-nums text-destructive">{{ fmtCount(t.count) }}</span>
        </button>
      </div>

      <div class="p-4">
        <!-- overview -->
        <div v-if="tab === 'overview'" class="space-y-4">
          <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-6">
            <AppMetricCard label="Documents" :value="fmtCount(st.documents)" helper="Vectors in the collection now."><Boxes /></AppMetricCard>
            <AppMetricCard label="Behind" :value="st.lag?.total == null ? '—' : fmtCount(st.lag.total)" :tone="lagTone" helper="Changes in the topics not embedded yet (consumer lag)."><Gauge /></AppMetricCard>
            <AppMetricCard label="Changes read" :value="fmtCount(c.events)" helper="Kafka records since the sidecar started this sync."><Activity /></AppMetricCard>
            <AppMetricCard label="Upserts / deletes" :value="`${fmtCount(c.upserted)} / ${fmtCount(c.deleted)}`" helper="After batch compaction: several changes to one row in a batch become one write."><Zap /></AppMetricCard>
            <AppMetricCard label="Embedding" :value="st.embedding?.avg_ms ? `${Math.round(st.embedding.avg_ms)} ms` : '—'" helper="Average time per embedding request."><Sparkles /></AppMetricCard>
            <AppMetricCard label="Dead letters" :value="fmtCount(c.dead_letters_stored)" :tone="c.dead_letters_stored ? 'danger' : 'neutral'" helper="Rows that could not become documents. The sync keeps going."><AlertOctagon /></AppMetricCard>
          </div>

          <p class="flex items-center gap-2 text-sm text-muted-foreground">
            <Clock class="h-4 w-4" :stroke-width="1.75" />
            {{ st.message || "—" }}<template v-if="st.last_event_at"> · last change {{ ago(st.last_event_at) }}</template>
            <template v-if="sync.generation"> · build {{ sync.generation + 1 }}</template>
          </p>

          <div v-if="st.state === 'retrying' && st.last_error" class="alert-warning text-sm">
            <p class="font-medium">Waiting for a dependency — nothing is skipped or lost.</p>
            <p class="mt-1 break-words font-mono text-xs">{{ st.last_error }}</p>
            <p class="mt-1 text-xs">The batch is retried with back-off and its offsets are not committed until it is written.</p>
          </div>
          <div v-if="st.retention_trimmed?.length" class="alert-warning text-sm">
            <AlertTriangle class="mr-1 inline h-4 w-4" :stroke-width="1.75" />
            {{ st.retention_trimmed.map(tableOf).join(", ") }}: topic retention had already deleted older records when this build started, so those rows
            are missing from the collection. Run <b>Resync</b> on the pipeline to re-snapshot them; this sync picks them up automatically.
          </div>
          <div v-if="st.missing_topics?.length" class="alert-info text-sm">
            Waiting for topics that do not exist yet: <span class="font-mono">{{ st.missing_topics.join(", ") }}</span>. Start the pipeline (or wait for its snapshot).
          </div>

          <div class="grid gap-4 lg:grid-cols-2">
            <div>
              <AppSectionTitle title="Tables" tag="h3" />
              <table class="data-table mt-2">
                <thead><tr><th>Table</th><th>Topic</th><th class="text-right">Behind</th></tr></thead>
                <tbody>
                  <tr v-for="t in sync.topics" :key="t">
                    <td class="font-medium">{{ tableOf(t) }}</td>
                    <td class="font-mono text-xs text-muted-foreground">{{ t }}</td>
                    <td class="text-right tabular-nums">{{ st.lag?.topics?.[t] ?? "—" }}</td>
                  </tr>
                </tbody>
              </table>
              <p v-if="!topicLag.length && running" class="mt-2 text-xs text-muted-foreground">Lag appears once the consumer has joined its group.</p>
            </div>
            <div v-if="st.recent_errors?.length">
              <AppSectionTitle title="Recent problems" tag="h3" />
              <ul class="mt-2 space-y-2">
                <li v-for="(e, i) in st.recent_errors.slice(0, 6)" :key="i" class="rounded-lg border border-border px-3 py-2">
                  <p class="text-[11px] text-muted-foreground">{{ ago(e.at) }}</p>
                  <p class="break-words font-mono text-xs text-foreground">{{ e.error }}</p>
                </li>
              </ul>
            </div>
          </div>
        </div>

        <!-- search -->
        <div v-else-if="tab === 'search'" class="space-y-4">
          <form class="flex flex-wrap gap-2" @submit.prevent="runSearch">
            <div class="relative min-w-[16rem] flex-1">
              <Search class="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
              <input v-model="query" class="auth-input pl-8" placeholder="Ask in plain words, e.g. “late deliveries to Chiang Mai”" autofocus />
            </div>
            <select v-model="tableFilter" class="auth-input w-auto" title="Only this table">
              <option value="">All tables</option>
              <option v-for="t in sync.topics" :key="t" :value="tableOf(t)">{{ tableOf(t) }}</option>
            </select>
            <select v-model.number="k" class="auth-input w-auto" title="Results">
              <option v-for="n in [3, 5, 10, 20]" :key="n" :value="n">Top {{ n }}</option>
            </select>
            <button type="submit" class="btn-primary w-auto" :disabled="searching || !query.trim()">
              <component :is="searching ? Loader2 : Search" class="h-4 w-4" :class="searching && 'animate-spin'" :stroke-width="1.75" />Search
            </button>
          </form>
          <p class="text-xs text-muted-foreground">Uses the same model and collection your agents use — a quick way to check that retrieval returns the right rows.</p>
          <p v-if="searchError" class="alert-error text-sm">{{ searchError }}</p>
          <p v-if="hits && !hits.length" class="text-sm text-muted-foreground">No documents yet.</p>
          <p v-if="hits?.length" class="text-xs text-muted-foreground">{{ hits.length }} results in {{ took }} ms</p>
          <ol v-if="hits?.length" class="space-y-2">
            <li v-for="h in hits" :key="h.id" class="rounded-lg border border-border p-3">
              <div class="flex items-center gap-3">
                <div class="h-1.5 w-24 shrink-0 overflow-hidden rounded-full bg-muted" :title="`similarity ${h.score}`">
                  <div class="h-full rounded-full bg-primary" :style="{ width: `${Math.max(4, Math.min(100, h.score * 100))}%` }" />
                </div>
                <span class="text-xs tabular-nums text-muted-foreground">{{ h.score.toFixed(3) }}</span>
                <span class="truncate font-mono text-[11px] text-muted-foreground">{{ h.id }}</span>
              </div>
              <pre
                class="mt-2 cursor-pointer whitespace-pre-wrap break-words font-sans text-sm text-foreground"
                :class="!expanded.has(h.id) && 'line-clamp-4'"
                @click="expanded.has(h.id) ? expanded.delete(h.id) : expanded.add(h.id)"
              >{{ h.content }}</pre>
              <div class="mt-2 flex flex-wrap gap-1">
                <span v-for="[key, v] in userMeta(h.metadata)" :key="key" class="tag-chip font-mono text-[11px]">{{ key }}={{ String(v).slice(0, 40) }}</span>
              </div>
            </li>
          </ol>
        </div>

        <!-- dead letters -->
        <div v-else-if="tab === 'dead'" class="space-y-3">
          <div class="flex items-center justify-between gap-2">
            <p class="text-sm text-muted-foreground">
              Rows that could not become documents (no key, a template column that does not exist, text the model rejected). The sync commits past them and keeps going;
              row values are not copied here — read them from the topic at that offset.
            </p>
            <button type="button" class="btn-secondary w-auto shrink-0" :disabled="!dead?.total" @click="clearDead"><Trash2 class="h-4 w-4" :stroke-width="1.75" />Clear</button>
          </div>
          <div v-if="deadLoading && !dead" class="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 class="h-4 w-4 animate-spin" :stroke-width="1.75" />Loading…</div>
          <AppEmptyState v-else-if="!dead?.items.length" title="No dead letters" description="Every row so far became a document." />
          <table v-else class="data-table">
            <thead><tr><th>When</th><th>Table</th><th>Offset</th><th>Key</th><th>Problem</th></tr></thead>
            <tbody>
              <tr v-for="(d, i) in dead.items" :key="i">
                <td class="whitespace-nowrap text-xs text-muted-foreground">{{ ago(d.at) }}</td>
                <td class="text-xs">{{ tableOf(d.topic) }}</td>
                <td class="font-mono text-xs">{{ d.partition }}:{{ d.offset }}</td>
                <td class="max-w-[12rem] truncate font-mono text-xs" :title="d.key">{{ d.key || "—" }}</td>
                <td class="text-xs text-destructive">{{ d.error }}</td>
              </tr>
            </tbody>
          </table>
        </div>

        <!-- settings -->
        <div v-else class="grid gap-4 lg:grid-cols-2">
          <dl class="grid grid-cols-[9rem_1fr] gap-x-3 gap-y-2 text-sm">
            <dt class="text-muted-foreground">Pipeline</dt><dd>{{ pipelineName || "—" }}</dd>
            <dt class="text-muted-foreground">Start</dt><dd>{{ sync.start_from === "latest" ? "New changes only" : "Backfill, then follow changes" }}</dd>
            <dt class="text-muted-foreground">Model</dt><dd><span class="font-mono">{{ sync.embedding.model }}</span> ({{ sync.embedding.provider }})</dd>
            <dt class="text-muted-foreground">Server</dt><dd class="truncate font-mono text-xs">{{ sync.embedding.base_url || "—" }}</dd>
            <dt class="text-muted-foreground">Store</dt><dd class="truncate">{{ sync.target.username }}@{{ sync.target.host }}:{{ sync.target.port || 5432 }}/{{ sync.target.database }}</dd>
            <dt class="text-muted-foreground">Collection</dt><dd class="font-mono">{{ sync.target.collection }}</dd>
            <dt class="text-muted-foreground">Deletes</dt><dd>{{ sync.delete_mode === "soft" ? "Keep, marked _is_deleted" : "Remove the vector" }}</dd>
            <dt class="text-muted-foreground">Consumer group</dt><dd class="truncate font-mono text-xs">{{ st.group_id || "—" }}</dd>
          </dl>
          <div>
            <AppSectionTitle title="Use it from code" tag="h3" />
            <pre class="code-block mt-2 overflow-auto text-xs">{{ snippet }}</pre>
            <p class="mt-2 text-xs text-muted-foreground">Search with the same model, or the vectors will not match.</p>
          </div>
        </div>
      </div>
    </div>

    <AppConfirmDialog v-model:open="confirmRebuild" title="Rebuild the collection?" confirm-label="Rebuild" :loading="busy === 'rebuild'" @confirm="act('rebuild')">
      Deletes the vectors in <span class="font-mono">{{ sync.target.collection }}</span> and embeds everything the topics still hold again.
      <template v-if="st.retention_trimmed?.length"> Rows older than topic retention will not come back.</template>
    </AppConfirmDialog>
    <AppConfirmDialog v-model:open="confirmDelete" title="Delete this vector sync?" confirm-label="Delete" danger :loading="busy === 'delete'" @confirm="remove">
      <p>Stops it and removes its settings, consumer groups and dead letters.</p>
      <label class="mt-3 flex items-start gap-2 text-foreground">
        <input v-model="dropCollection" type="checkbox" class="mt-0.5 h-4 w-4 rounded border-border accent-brand-500" />
        <span>Also delete the collection and its {{ fmtCount(st.documents) }} vectors</span>
      </label>
    </AppConfirmDialog>
  </div>
</template>
