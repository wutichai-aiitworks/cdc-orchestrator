<script setup lang="ts">
import { Download, MoreVertical, RefreshCw, RotateCcw, Search, Trash2, Users } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import { downloadCsv } from "~/utils/csv"

const router = useRouter()
const route = useRoute()
const { fetchGroups } = useKafkaM1()
const { deleteGroup } = useKafkaAdmin()
const { getPref, setPref } = useViewPrefs()
const toast = useToast()

// ── Filter / pagination state ──────────────────────────────────────────────
const page = ref(1)
const pageSize = ref(50)
const search = ref("")
const stateFilter = ref("")
const loading = ref(false)
const error = ref("")
const initialized = ref(false)
const lastUpdated = ref("")

let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null

const groups = ref<{
  items: Array<{ group: string; state?: string | null; members?: number | null; total_lag?: number | null }>
  total: number
  page: number
  page_size: number
} | null>(null)

const totalPages = computed(() => Math.max(1, Math.ceil((groups.value?.total ?? 0) / pageSize.value)))

// ── Per-row action menu ────────────────────────────────────────────────────
const menuOpenFor = ref<string | null>(null)
const menuStyle = ref({ top: "0px", left: "0px" })
const menuBtnRefs = ref<Map<string, HTMLElement>>(new Map())

const setMenuRef = (el: any, group: string) => {
  if (el) menuBtnRefs.value.set(group, el as HTMLElement)
}

const openMenu = (groupName: string, e: MouseEvent) => {
  e.stopPropagation()
  menuOpenFor.value = groupName
  const btn = menuBtnRefs.value.get(groupName)
  if (btn) {
    const rect = btn.getBoundingClientRect()
    menuStyle.value = {
      top: `${rect.bottom + 4}px`,
      left: `${Math.max(8, rect.right - 180)}px`,
    }
  }
}

const closeMenu = () => { menuOpenFor.value = null }

// ── Delete ─────────────────────────────────────────────────────────────────
const pendingDelete = ref<string | null>(null)
const deleting = ref(false)

const confirmDelete = async () => {
  if (!pendingDelete.value) return
  const g = pendingDelete.value
  pendingDelete.value = null
  deleting.value = true
  try {
    await deleteGroup(g)
    toast.success(`Group "${g}" deleted`)
    await loadGroups()
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to delete group"))
  } finally {
    deleting.value = false
  }
}

// ── Load ───────────────────────────────────────────────────────────────────
const syncQuery = async () => {
  const q: Record<string, string> = { page: String(page.value), page_size: String(pageSize.value) }
  if (search.value.trim()) q.search = search.value.trim()
  if (stateFilter.value) q.state = stateFilter.value
  await router.replace({ query: q })
}

const loadGroups = async () => {
  await syncQuery()
  loading.value = true
  error.value = ""
  try {
    groups.value = await fetchGroups({
      page: page.value,
      page_size: pageSize.value,
      search: search.value.trim() || undefined,
      state: stateFilter.value || undefined,
    })
    lastUpdated.value = new Date().toLocaleTimeString()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load consumer groups")
  } finally {
    loading.value = false
  }
}

const applyFilters = async () => { page.value = 1; await loadGroups() }


const lagColor = (lag: number | null | undefined) => {
  const n = lag ?? 0
  if (n === 0) return "text-emerald-500 dark:text-emerald-400"
  if (n < 1000) return "text-yellow-600 dark:text-yellow-400"
  return "text-red-500 dark:text-red-400"
}

// Kafka infrastructure groups: true Kafka internals (__*) and
// Connect worker coordination groups (group.id, always ends with -cluster)
const isInternalGroup = (name: string) =>
  name.startsWith("__") ||
  name === "_schemas" ||
  name.endsWith("-cluster")

// ── Lifecycle ──────────────────────────────────────────────────────────────
onMounted(async () => {
  const defaultPageSize = Number(getPref("groups.pageSize", 50))
  page.value = Math.max(1, Number(route.query.page || 1))
  pageSize.value = Math.max(1, Number(route.query.page_size || defaultPageSize))
  search.value = String(route.query.search || "")
  stateFilter.value = String(route.query.state || "")
  await loadGroups()
  initialized.value = true
})

watch(pageSize, (v) => setPref("groups.pageSize", v))

watch(search, () => {
  if (!initialized.value) return
  if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
  searchDebounceTimer = setTimeout(() => applyFilters(), 350)
})

onUnmounted(() => { if (searchDebounceTimer) clearTimeout(searchDebounceTimer) })
</script>

<template>
  <div class="page-container max-w-6xl space-y-4">
    <!-- Header -->
    <AppPageHeader
      title="Consumer Groups"
      description="Offset lag, member counts, and group state across all consumer groups."
    >
      <AppButton :disabled="loading" @click="loadGroups">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton
        :disabled="!(groups?.items?.length)"
        @click="downloadCsv('consumer-groups', (groups?.items || []).map((g) => ({ group: g.group, state: g.state || '', members: g.members ?? '', lag: g.total_lag ?? '' })))"
      >
        <Download class="h-4 w-4" :stroke-width="1.75" />
        Export CSV
      </AppButton>
    </AppPageHeader>

    <!-- Toolbar -->
    <AppCard class="!p-3">
      <div class="flex flex-wrap items-center gap-2">
        <!-- Search -->
        <div class="relative flex-1 min-w-48">
          <Search class="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
          <AppInput v-model="search" input-class="pl-9" placeholder="Search groups…" @keyup.enter="applyFilters" />
        </div>
        <!-- State filter -->
        <AppSelect v-model="stateFilter" class="w-36" @change="applyFilters">
          <option value="">All States</option>
          <option value="stable">Stable</option>
          <option value="empty">Empty</option>
          <option value="dead">Dead</option>
          <option value="rebalancing">Rebalancing</option>
        </AppSelect>
        <!-- Rows per page -->
        <AppSelect
          :model-value="pageSize"
          class="w-28"
          @update:model-value="pageSize = Number($event); applyFilters()"
        >
          <option :value="25">25 rows</option>
          <option :value="50">50 rows</option>
          <option :value="100">100 rows</option>
        </AppSelect>
        <span v-if="lastUpdated" class="ml-auto text-xs text-muted-foreground">Updated {{ lastUpdated }}</span>
      </div>
    </AppCard>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <!-- Table -->
    <AppCard class="overflow-hidden !p-0">
      <!-- Table header -->
      <div class="grid grid-cols-[1fr_70px_80px_110px_100px_100px_44px] border-b border-border bg-surface-900/60 px-4 py-2.5 text-xs font-medium text-muted-foreground">
        <div>Group Name</div>
        <div class="text-center">Topics</div>
        <div class="text-center">Members</div>
        <div>Coordinator</div>
        <div>State</div>
        <div class="text-right">Consumer Lag</div>
        <div />
      </div>

      <!-- Loading -->
      <div v-if="loading && !groups" class="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground">
        <RefreshCw class="h-4 w-4 animate-spin opacity-50" :stroke-width="1.75" />
        Loading…
      </div>

      <!-- Empty -->
      <div v-else-if="!loading && (groups?.items?.length ?? 0) === 0" class="py-10 text-center text-sm text-muted-foreground">
        <Users class="mx-auto mb-2 h-8 w-8 opacity-30" :stroke-width="1.5" />
        No consumer groups found.
      </div>

      <!-- Rows -->
      <template v-else>
        <div
          v-for="g in groups?.items ?? []"
          :key="g.group"
          class="group grid grid-cols-[1fr_70px_80px_110px_100px_100px_44px] items-center border-b border-border/50 px-4 py-2.5 transition-colors hover:bg-accent/30"
        >
          <!-- Name -->
          <div class="flex min-w-0 items-center gap-2 overflow-hidden">
            <NuxtLink
              :to="`/consumer-groups/${encodeURIComponent(g.group)}`"
              class="truncate font-mono text-sm font-medium hover:text-brand-500"
            >{{ g.group }}</NuxtLink>
            <span
              v-if="isInternalGroup(g.group)"
              class="inline-flex flex-shrink-0 items-center rounded border border-blue-500/30 bg-blue-500/10 px-1.5 py-0.5 text-xs font-medium text-blue-600 dark:text-blue-400"
              title="Kafka Connect or Kafka infrastructure group — not a user consumer group"
            >Internal</span>
          </div>
          <!-- Topics -->
          <div class="text-center font-mono text-sm text-muted-foreground">{{ g.topic_count ?? '—' }}</div>
          <!-- Members -->
          <div class="text-center font-mono text-sm text-muted-foreground">{{ g.members ?? '—' }}</div>
          <!-- Coordinator -->
          <div class="font-mono text-sm text-muted-foreground">{{ g.coordinator ? `Broker ${g.coordinator}` : '—' }}</div>
          <!-- State -->
          <div>
            <AppStatusBadge v-if="g.state" :status="g.state" />
            <span v-else class="text-muted-foreground text-xs">—</span>
          </div>
          <!-- Consumer Lag -->
          <div class="text-right font-mono text-sm font-semibold" :class="lagColor(g.total_lag)">
            {{ g.total_lag ?? '—' }}
          </div>
          <!-- 3-dots menu -->
          <div class="flex items-center justify-end">
            <button
              :ref="(el) => setMenuRef(el, g.group)"
              type="button"
              class="invisible inline-flex h-7 w-7 items-center justify-center rounded-lg text-muted-foreground hover:bg-accent hover:text-foreground group-hover:visible"
              :class="{ '!visible': menuOpenFor === g.group }"
              @click="openMenu(g.group, $event)"
            >
              <MoreVertical class="h-3.5 w-3.5" :stroke-width="1.75" />
            </button>
          </div>
        </div>
      </template>

      <!-- Pagination -->
      <div v-if="(groups?.total ?? 0) > 0" class="flex items-center justify-between border-t border-border/50 px-4 py-2.5 text-xs text-muted-foreground">
        <span>{{ groups?.total }} group{{ (groups?.total ?? 0) !== 1 ? 's' : '' }}</span>
        <div class="flex items-center gap-1">
          <button
            class="rounded px-2 py-1 hover:bg-accent disabled:opacity-40"
            :disabled="page <= 1"
            @click="page -= 1; loadGroups()"
          >← Prev</button>
          <span class="px-2">{{ page }} / {{ totalPages }}</span>
          <button
            class="rounded px-2 py-1 hover:bg-accent disabled:opacity-40"
            :disabled="page >= totalPages"
            @click="page += 1; loadGroups()"
          >Next →</button>
        </div>
      </div>
    </AppCard>

    <!-- Action menu (Teleport to body to avoid overflow clipping) -->
    <Teleport to="body">
      <div
        v-if="menuOpenFor"
        class="fixed z-[9999] min-w-[180px] rounded-xl border border-border bg-popover p-1 shadow-2xl"
        :style="menuStyle"
        @click.stop
      >
        <!-- Invisible backdrop to close on outside click -->
        <div class="fixed inset-0 -z-10" @click="closeMenu" />

        <div class="px-3 py-1.5 text-xs font-medium text-muted-foreground truncate max-w-[220px]">
          {{ menuOpenFor }}
        </div>
        <div class="my-0.5 border-t border-border/50" />

        <!-- View Details -->
        <NuxtLink
          :to="`/consumer-groups/${encodeURIComponent(menuOpenFor)}`"
          class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm font-medium text-foreground hover:bg-accent"
          @click="closeMenu"
        >
          <Users class="h-3.5 w-3.5 flex-shrink-0" :stroke-width="1.75" />
          View Details
        </NuxtLink>

        <!-- Reset Offsets -->
        <NuxtLink
          :to="`/consumer-groups/${encodeURIComponent(menuOpenFor)}/reset`"
          class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm font-medium text-foreground hover:bg-accent"
          @click="closeMenu"
        >
          <RotateCcw class="h-3.5 w-3.5 flex-shrink-0" :stroke-width="1.75" />
          Reset Offsets
        </NuxtLink>

        <div class="my-0.5 border-t border-border/50" />

        <!-- Delete -->
        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm font-medium text-red-500 hover:bg-red-500/10"
          @click="pendingDelete = menuOpenFor; closeMenu()"
        >
          <Trash2 class="h-3.5 w-3.5 flex-shrink-0" :stroke-width="1.75" />
          Delete Group
        </button>
      </div>
    </Teleport>

    <!-- Delete confirm -->
    <AppConfirmDialog
      :open="!!pendingDelete"
      title="Delete consumer group?"
      danger
      confirm-label="Delete"
      :loading="deleting"
      @update:open="(v) => { if (!v) pendingDelete = null }"
      @confirm="confirmDelete"
    >
      Delete group <strong class="font-mono">{{ pendingDelete }}</strong>? This cannot be undone.
    </AppConfirmDialog>
  </div>
</template>
