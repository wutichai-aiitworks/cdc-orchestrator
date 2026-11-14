<script setup lang="ts">
// AI Vector Sync (ADR 0018): keep a PGVector collection in step with CDC tables so AI agents and
// RAG search always read current data. List → detail page via the URL (?sync=<id>), editor via
// ?new=1 / ?sync=<id>&edit=1. Behind VECTORSYNC_ENABLED (utils/featurePages.ts).
import { Boxes, Database, FlaskConical, Plus, RefreshCw, Search, Sparkles } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import type { VsCapability, VsProfile } from "~/composables/useVectorSync"

definePageMeta({ middleware: "feature" })

const api = useVectorSync()
const route = useRoute()
const router = useRouter()

const capability = ref<VsCapability | null>(null)
const syncs = ref<VsProfile[]>([])
const sidecarReady = ref(true)
const loading = ref(true)
const error = ref("")
const pipelines = ref<Record<string, string>>({})

const syncId = computed(() => (typeof route.query.sync === "string" ? route.query.sync : ""))
const mode = computed<"list" | "detail" | "new" | "edit">(() =>
  route.query.new ? "new" : syncId.value && route.query.edit ? "edit" : syncId.value ? "detail" : "list",
)
const current = ref<VsProfile | null>(null)

const loadList = async () => {
  try {
    const r = await api.list()
    syncs.value = r.profiles
    sidecarReady.value = r.sidecar_ready
    error.value = ""
  } catch (e: any) {
    error.value = formatApiError(e, "Could not load vector syncs")
  } finally {
    loading.value = false
  }
}
const loadCurrent = async () => {
  if (!syncId.value) return (current.value = null)
  try {
    current.value = await api.get(syncId.value)
  } catch (e: any) {
    error.value = formatApiError(e, "Could not load this vector sync")
  }
}
const loadPipelines = async () => {
  try {
    const r = await $fetch<{ profiles: { id: string; name: string }[] }>("/api/cdc/profiles")
    pipelines.value = Object.fromEntries(r.profiles.map((p) => [p.id, p.name]))
  } catch {
    /* names are cosmetic */
  }
}
const refresh = () => (mode.value === "list" ? loadList() : loadCurrent())

const open = (id: string) => router.push({ query: { sync: id } })
const toList = () => router.push({ query: {} })

const onSaved = (p: VsProfile) => {
  current.value = p
  void router.replace({ query: { sync: p.id } })
  void loadList()
}

watch(syncId, () => void loadCurrent(), { immediate: true })

let timer: ReturnType<typeof setInterval> | null = null
onMounted(async () => {
  capability.value = await api.capability().catch(() => null)
  void loadList()
  void loadPipelines()
  timer = setInterval(() => {
    if (document.visibilityState !== "visible") return
    if (mode.value === "list") void loadList()
    else if (mode.value === "detail") void loadCurrent()
  }, 5000)
})
onBeforeUnmount(() => timer && clearInterval(timer))

// Breadcrumbs: AI Vector Sync › <name> › Edit / New
const crumbs = usePageCrumbs()
watchEffect(() => {
  crumbs.value = mode.value === "new" ? ["New sync"] : current.value && mode.value !== "list" ? [current.value.name, ...(mode.value === "edit" ? ["Edit"] : [])] : []
})
onBeforeUnmount(() => (crumbs.value = []))
const { onNavReselect } = useNavReselect()
onNavReselect("/vectorsync", toList)

const totals = computed(() => ({
  docs: syncs.value.reduce((a, s) => a + (s.status?.documents || 0), 0),
  running: syncs.value.filter((s) => s.status?.state === "running").length,
  attention: syncs.value.filter((s) => ["retrying", "error", "unknown"].includes(s.status?.state) || (s.status?.counters?.dead_letters_stored || 0) > 0).length,
}))
const versionNote = computed(() => {
  const v = capability.value?.versions
  return v ? `pydebeziumai ${v.pydebeziumai} · langchain-postgres ${v.langchain_postgres}` : ""
})
</script>

<template>
  <div class="page-container space-y-4">
    <!-- editor -->
    <template v-if="mode === 'new' || mode === 'edit'">
      <AppPageHeader
        :title="mode === 'new' ? 'New vector sync' : `Edit ${current?.name || ''}`"
        description="Pick tables, choose an embedding model and a pgvector store, then shape how rows read as documents."
      />
      <VsWizard v-if="mode === 'new' || current" :key="mode + (current?.id || '')" :profile="mode === 'edit' ? current : null" :capability="capability" @saved="onSaved" @cancel="mode === 'edit' ? open(syncId) : toList()" />
    </template>

    <!-- detail -->
    <template v-else-if="mode === 'detail'">
      <p v-if="error && !current" class="alert-error text-sm">{{ error }}</p>
      <AppStateBlock v-else-if="!current" kind="loading" message="Loading…" />
      <VsDetail
        v-else
        :sync="current"
        :pipeline-name="current.source_profile_id ? pipelines[current.source_profile_id] : undefined"
        @edit="router.push({ query: { sync: current.id, edit: '1' } })"
        @changed="(p) => (p ? (current = p) : loadCurrent())"
        @deleted="toList(); loadList()"
      />
    </template>

    <!-- list -->
    <template v-else>
      <AppPageHeader title="AI Vector Sync" description="Keep vector collections in step with your CDC tables, so AI agents and RAG search always answer from current data.">
        <span v-if="versionNote" class="hidden text-xs text-muted-foreground md:inline" :title="'pydebeziumai is a first release; this feature uses only its document builder and PGVector adapter, with delivery handled by the sidecar (ADR 0018).'">
          <FlaskConical class="mr-1 inline h-3.5 w-3.5" :stroke-width="1.75" />{{ versionNote }}
        </span>
        <button type="button" class="icon-button" title="Refresh" @click="refresh"><RefreshCw class="h-4 w-4" :stroke-width="1.75" /></button>
        <button type="button" class="btn-primary w-auto" :disabled="capability?.sidecar_ready === false" @click="router.push({ query: { new: '1' } })"><Plus class="h-4 w-4" :stroke-width="1.75" />New sync</button>
      </AppPageHeader>

      <div v-if="capability && capability.sidecar_ready === false" class="alert-warning text-sm">
        <p class="font-medium">The vectorsync sidecar is not answering.</p>
        <p class="mt-1 text-xs">{{ capability.hint }} <span class="font-mono">{{ capability.error }}</span></p>
      </div>
      <p v-else-if="!sidecarReady && syncs.length" class="alert-warning text-sm">Live status is unavailable — the sidecar did not answer. Syncs resume when it is back.</p>
      <p v-if="error" class="alert-error text-sm">{{ error }}</p>

      <AppStateBlock v-if="loading" kind="loading" message="Loading…" />

      <!-- empty: explain the flow in three steps -->
      <section v-else-if="!syncs.length" class="stat-card py-10">
        <div class="mx-auto max-w-3xl text-center">
          <Sparkles class="mx-auto h-8 w-8 text-primary" :stroke-width="1.5" />
          <h2 class="mt-3 text-lg font-semibold text-foreground">Give your AI agents live data</h2>
          <p class="mt-1 text-sm text-muted-foreground">
            Every insert, update and delete your pipelines capture is turned into an up-to-date vector — no batch re-indexing, no stale answers.
          </p>
          <div class="mt-8 grid gap-4 text-left sm:grid-cols-3">
            <div class="rounded-lg border border-border p-4">
              <Database class="h-5 w-5 text-muted-foreground" :stroke-width="1.75" />
              <p class="mt-2 text-sm font-medium text-foreground">1. Pick tables</p>
              <p class="mt-1 text-xs text-muted-foreground">From a running CDC pipeline. The sync reads its topics, not your database.</p>
            </div>
            <div class="rounded-lg border border-border p-4">
              <Boxes class="h-5 w-5 text-muted-foreground" :stroke-width="1.75" />
              <p class="mt-2 text-sm font-medium text-foreground">2. Shape documents</p>
              <p class="mt-1 text-xs text-muted-foreground">Choose the text to embed, preview real rows, keep credentials out.</p>
            </div>
            <div class="rounded-lg border border-border p-4">
              <Search class="h-5 w-5 text-muted-foreground" :stroke-width="1.75" />
              <p class="mt-2 text-sm font-medium text-foreground">3. Search</p>
              <p class="mt-1 text-xs text-muted-foreground">Vectors land in pgvector, ready for LangChain, LangGraph or SQL.</p>
            </div>
          </div>
          <button type="button" class="btn-primary w-auto mt-8" :disabled="capability?.sidecar_ready === false" @click="router.push({ query: { new: '1' } })"><Plus class="h-4 w-4" :stroke-width="1.75" />Create your first sync</button>
        </div>
      </section>

      <template v-else>
        <div class="grid gap-3 sm:grid-cols-3">
          <AppMetricCard label="Vector syncs" :value="syncs.length" :helper="`${totals.running} running`"><Sparkles /></AppMetricCard>
          <AppMetricCard label="Documents" :value="totals.docs.toLocaleString('en-US')" helper="Across all collections."><Boxes /></AppMetricCard>
          <AppMetricCard label="Need attention" :value="totals.attention" :tone="totals.attention ? 'warning' : 'neutral'" helper="Retrying, unreachable, or with dead letters."><Search /></AppMetricCard>
        </div>
        <div class="grid gap-3 md:grid-cols-2 2xl:grid-cols-3">
          <VsSyncCard v-for="s in syncs" :key="s.id" :sync="s" :pipeline-name="s.source_profile_id ? pipelines[s.source_profile_id] : undefined" @open="open" />
        </div>
      </template>
    </template>
  </div>
</template>
