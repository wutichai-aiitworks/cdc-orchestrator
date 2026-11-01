<script setup lang="ts">
import { Eye, Plus, RefreshCw, Search } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import { formatBytes } from "~/utils/bytes"
import { downloadCsv } from "~/utils/csv"
import type { TopicItem, TopicSortBy } from "~/composables/useKafkaM1"

const route = useRoute()
const router = useRouter()
const toast = useToast()
const { fetchTopics, deleteTopic, recreateTopic, clearTopicMessages } = useKafkaM1()
const { getPref, setPref } = useViewPrefs()

const page = ref(1)
const pageSize = ref(25)
const search = ref("")
const hideInternal = ref(false)
const sortBy = ref<TopicSortBy>("name")
const sortOrder = ref<"asc" | "desc">("asc")
const loading = ref(false)
const actionLoading = ref(false)
const error = ref("")
const result = ref<{
  items: TopicItem[]
  total: number
  page: number
  page_size: number
  timestamp?: string
} | null>(null)
const selected = ref<string[]>([])
const initialized = ref(false)
const lastUpdated = ref("")
let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null

const peekTopic = ref<string | null>(null)

const confirm = ref<{
  kind: "clear" | "recreate" | "remove" | "batch-delete" | "batch-purge"
  topics: string[]
} | null>(null)

const toPositiveInt = (value: unknown, fallback: number) => {
  const n = Number(value)
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : fallback
}

const isSortBy = (value: string): value is TopicSortBy =>
  ["name", "partitions", "out_of_sync_replicas", "replication_factor", "messages_count", "size"].includes(value)

const syncQuery = async () => {
  const query: Record<string, string> = {
    page: String(page.value),
    page_size: String(pageSize.value),
    sort_by: sortBy.value,
    sort_order: sortOrder.value,
  }
  if (search.value.trim()) query.search = search.value.trim()
  if (hideInternal.value) query.hide_internal = "true"
  await router.replace({ query })
}

const load = async () => {
  await syncQuery()
  loading.value = true
  error.value = ""
  try {
    result.value = await fetchTopics({
      page: page.value,
      page_size: pageSize.value,
      search: search.value.trim() || undefined,
      hide_internal: hideInternal.value,
      sort_by: sortBy.value,
      sort_order: sortOrder.value,
    })
    const visible = new Set((result.value.items || []).map((item) => item.name))
    selected.value = selected.value.filter((name) => visible.has(name))
    lastUpdated.value = new Date().toLocaleTimeString()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load topics")
  } finally {
    loading.value = false
  }
}

const totalPages = computed(() => {
  if (!result.value) return 1
  return Math.max(1, Math.ceil(result.value.total / result.value.page_size))
})

const visibleNames = computed(() => (result.value?.items || []).map((item) => item.name))
const allVisibleSelected = computed(
  () => visibleNames.value.length > 0 && visibleNames.value.every((name) => selected.value.includes(name))
)
const selectedTopics = computed(
  () => (result.value?.items || []).filter((item) => selected.value.includes(item.name))
)

const applyFilters = async () => {
  page.value = 1
  await load()
}

const toggleSort = async (column: string) => {
  if (!isSortBy(column)) return
  if (sortBy.value === column) {
    sortOrder.value = sortOrder.value === "asc" ? "desc" : "asc"
  } else {
    sortBy.value = column
    sortOrder.value = "asc"
  }
  page.value = 1
  await load()
}

const toggleAll = () => {
  if (allVisibleSelected.value) {
    selected.value = []
    return
  }
  selected.value = [...visibleNames.value]
}

const toggleOne = (name: string) => {
  selected.value = selected.value.includes(name)
    ? selected.value.filter((item) => item !== name)
    : [...selected.value, name]
}

const confirmTitle = computed(() => {
  const topic = confirm.value?.topics[0] || "this topic"
  switch (confirm.value?.kind) {
    case "clear":
      return `Clear messages in ${topic}?`
    case "recreate":
      return `Are you sure to recreate ${topic} topic?`
    case "remove":
      return `Are you sure want to remove ${topic} topic?`
    case "batch-delete":
      return "Are you sure you want to remove selected topics?"
    case "batch-purge":
      return "Are you sure you want to purge messages of selected topics?"
    default:
      return "Confirm"
  }
})

const runConfirmed = async () => {
  if (!confirm.value) return
  const { kind, topics } = confirm.value
  confirm.value = null
  actionLoading.value = true
  error.value = ""
  try {
    if (kind === "clear" || kind === "batch-purge") {
      await Promise.all(topics.map((name) => clearTopicMessages(name)))
      toast.success(topics.length === 1 ? `Cleared messages in ${topics[0]}` : `Purged ${topics.length} topics`)
    } else if (kind === "recreate") {
      await recreateTopic(topics[0])
      toast.success(`Recreated ${topics[0]}`)
    } else {
      await Promise.all(topics.map((name) => deleteTopic(name)))
      toast.success(topics.length === 1 ? `Removed ${topics[0]}` : `Removed ${topics.length} topics`)
      selected.value = selected.value.filter((name) => !topics.includes(name))
    }
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Topic action failed")
    toast.error(error.value)
  } finally {
    actionLoading.value = false
  }
}

onMounted(async () => {
  const defaultPageSize = Number(getPref("topics.pageSize", 25))
  page.value = toPositiveInt(route.query.page, 1)
  pageSize.value = toPositiveInt(route.query.page_size, defaultPageSize)
  search.value = String(route.query.search || "")
  hideInternal.value =
    route.query.hide_internal != null
      ? String(route.query.hide_internal) === "true"
      : Boolean(getPref("topics.hideInternal", false))
  const requestedSort = String(route.query.sort_by || getPref("topics.sortBy", "name"))
  sortBy.value = isSortBy(requestedSort) ? requestedSort : "name"
  sortOrder.value = String(route.query.sort_order || getPref("topics.sortOrder", "asc")) === "desc" ? "desc" : "asc"
  await load()
  initialized.value = true
})

watch(pageSize, (v) => setPref("topics.pageSize", v))
watch(hideInternal, (v) => {
  if (!initialized.value) return
  setPref("topics.hideInternal", v)
  applyFilters()
})
watch([sortBy, sortOrder], ([by, order]) => {
  setPref("topics.sortBy", by)
  setPref("topics.sortOrder", order)
})
watch(search, () => {
  if (!initialized.value) return
  if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
  searchDebounceTimer = setTimeout(() => {
    applyFilters()
  }, 350)
})

onUnmounted(() => {
  if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
})
</script>

<template>
  <div class="page-container max-w-7xl">
    <AppPageHeader
      title="Topics"
      description="Create topics, inspect partitions and storage, and manage messages the same way as Kafbat UI."
    >
      <AppButton :disabled="loading || actionLoading" @click="load">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton as="link" to="/topics/create" variant="primary">
        <Plus class="h-4 w-4" :stroke-width="1.75" />
        Add a Topic
      </AppButton>
      <AppButton
        :disabled="!(result?.items?.length)"
        @click="downloadCsv('topics', (result?.items || []).map((item) => ({ name: item.name, partitions: item.partition_count, outOfSync: item.out_of_sync_replicas, replication: item.replication_factor, messages: item.messages_count ?? '', size: item.segment_size ?? '' })))"
      >
        Export CSV
      </AppButton>
    </AppPageHeader>

    <AppCard>
      <div class="grid grid-cols-1 items-end gap-3 md:grid-cols-[minmax(0,1fr),180px,auto]">
        <label>
          <span class="auth-label">Search Topics</span>
          <div class="relative">
            <Search class="pointer-events-none absolute left-3 top-3 h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
            <AppInput v-model="search" input-class="pl-9" placeholder="Search by topic name..." @enter="applyFilters" />
          </div>
        </label>
        <label>
          <span class="auth-label">Rows per Page</span>
          <AppSelect
            :model-value="pageSize"
            aria-label="Rows per page"
            @update:model-value="pageSize = Number($event); applyFilters()"
          >
            <option :value="25">25</option>
            <option :value="50">50</option>
            <option :value="100">100</option>
          </AppSelect>
        </label>
        <label class="mb-2 inline-flex items-center gap-2 text-sm text-muted-foreground">
          <input v-model="hideInternal" type="checkbox" class="accent-brand-500" />
          Hide Internal Topics
        </label>
      </div>
    </AppCard>

    <AppStateBlock v-if="error" kind="error" :message="error" />
    <AppStateBlock v-else-if="loading && !result" kind="loading" message="Loading topics..." />

    <AppCard>
      <div class="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 class="section-title">Kafka Topics</h2>
          <AppResultsHeader
            label="Topics"
            :total="result?.total || 0"
            :page="page"
            :total-pages="totalPages"
            :loading="loading"
            :last-updated="lastUpdated"
          />
        </div>
        <div v-if="selected.length" class="flex flex-wrap gap-2">
          <AppButton
            variant="destructive"
            :disabled="actionLoading || selectedTopics.some((item) => item.internal)"
            @click="confirm = { kind: 'batch-delete', topics: [...selected] }"
          >
            Delete selected topics
          </AppButton>
          <AppButton
            :disabled="actionLoading || selectedTopics.some((item) => item.internal || (item.cleanup_policy || 'delete').toLowerCase() !== 'delete')"
            @click="confirm = { kind: 'batch-purge', topics: [...selected] }"
          >
            Purge messages of selected topics
          </AppButton>
        </div>
      </div>

      <AppTable>
        <thead>
          <tr>
            <th class="w-10">
              <input
                type="checkbox"
                class="accent-brand-500"
                :checked="allVisibleSelected"
                :disabled="!visibleNames.length"
                @change="toggleAll"
              />
            </th>
            <th><AppSortHeader label="Topic Name" column="name" :sort-by="sortBy" :sort-order="sortOrder" @sort="toggleSort" /></th>
            <th><AppSortHeader label="Partitions" column="partitions" :sort-by="sortBy" :sort-order="sortOrder" @sort="toggleSort" /></th>
            <th><AppSortHeader label="Out of sync replicas" column="out_of_sync_replicas" :sort-by="sortBy" :sort-order="sortOrder" @sort="toggleSort" /></th>
            <th><AppSortHeader label="Replication Factor" column="replication_factor" :sort-by="sortBy" :sort-order="sortOrder" @sort="toggleSort" /></th>
            <th><AppSortHeader label="Number of messages" column="messages_count" :sort-by="sortBy" :sort-order="sortOrder" @sort="toggleSort" /></th>
            <th><AppSortHeader label="Size" column="size" :sort-by="sortBy" :sort-order="sortOrder" @sort="toggleSort" /></th>
            <th class="w-12" />
          </tr>
        </thead>
        <tbody>
          <tr v-for="item in result?.items || []" :key="item.name" class="group" :class="{ 'bg-accent': selected.includes(item.name) }">
            <td>
              <input
                type="checkbox"
                class="accent-brand-500"
                :checked="selected.includes(item.name)"
                @change="toggleOne(item.name)"
              />
            </td>
            <td>
              <NuxtLink
                :to="`/topics/${encodeURIComponent(item.name)}`"
                class="inline-flex items-center gap-2 font-semibold text-foreground hover:text-brand-500"
              >
                <span v-if="item.internal" class="badge-neutral !rounded-md !px-1.5 !py-0.5 !text-[10px] font-bold tracking-wide">IN</span>
                <span>{{ item.name }}</span>
              </NuxtLink>
            </td>
            <td>{{ item.partition_count }}</td>
            <td>{{ item.out_of_sync_replicas }}</td>
            <td>{{ item.replication_factor }}</td>
            <td>{{ item.messages_count == null ? "N/A" : item.messages_count }}</td>
            <td>{{ formatBytes(item.segment_size) }}</td>
            <td>
              <div class="flex items-center justify-end gap-1">
                <button
                  v-if="!item.internal"
                  type="button"
                  title="Peek last message"
                  class="invisible icon-button text-surface-400 hover:text-brand-400 group-hover:visible"
                  @click.stop="peekTopic = item.name"
                >
                  <Eye class="h-4 w-4" :stroke-width="1.75" />
                </button>
                <TopicActionsMenu
                  :topic="item"
                  :disabled="actionLoading"
                  @clear="confirm = { kind: 'clear', topics: [$event.name] }"
                  @recreate="confirm = { kind: 'recreate', topics: [$event.name] }"
                  @remove="confirm = { kind: 'remove', topics: [$event.name] }"
                />
              </div>
            </td>
          </tr>
          <tr v-if="!loading && (result?.items?.length || 0) === 0">
            <td colspan="8" class="py-4">
              <AppStateBlock kind="empty" message="No topics found." />
            </td>
          </tr>
        </tbody>
      </AppTable>
      <div v-if="result" class="mt-4 flex items-center justify-between gap-3 border-t border-border pt-4">
        <p class="text-sm text-muted-foreground">Showing {{ result.items.length }} of {{ result.total }} topics</p>
        <AppPager
          :page="page"
          :total-pages="totalPages"
          :disabled="loading"
          @prev="page -= 1; load()"
          @next="page += 1; load()"
        />
      </div>
    </AppCard>

    <AppConfirmDialog
      :open="Boolean(confirm)"
      :title="confirmTitle"
      :danger="true"
      confirm-label="Confirm"
      @update:open="(value) => { if (!value) confirm = null }"
      @confirm="runConfirmed"
    >
      This action cannot be undone.
    </AppConfirmDialog>

    <AppPeekMessage :topic="peekTopic" @close="peekTopic = null" />
  </div>
</template>
