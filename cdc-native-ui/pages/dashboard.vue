<script setup lang="ts">
import { Download, RefreshCw } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import { downloadCsv } from "~/utils/csv"
import { formatBytes } from "~/utils/bytes"

const { fetchBrokers } = useKafkaAdmin()
const { fetchTopics, fetchGroups } = useKafkaM1()

const loading = ref(false)
const error = ref("")
const lastUpdated = ref("")
const brokers = ref<Awaited<ReturnType<typeof fetchBrokers>> | null>(null)
const topicCount = ref(0)
const groupCount = ref(0)

const refresh = async () => {
  loading.value = true
  error.value = ""
  try {
    const [brokerData, topics, groups] = await Promise.all([
      fetchBrokers(),
      fetchTopics({ page: 1, page_size: 1 }),
      fetchGroups({ page: 1, page_size: 1 }),
    ])
    brokers.value = brokerData
    topicCount.value = topics.total
    groupCount.value = groups.total
    lastUpdated.value = new Date().toLocaleTimeString()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load cluster dashboard")
  } finally {
    loading.value = false
  }
}

const exportCsv = () => {
  downloadCsv(
    "cluster-dashboard",
    (brokers.value?.items || []).map((item) => ({
      id: item.id,
      host: item.host,
      port: item.port,
      controller: item.controller,
      diskUsage: item.segment_size ?? "",
      segments: item.segment_count ?? "",
    }))
  )
}

const openBroker = async (id: number) => {
  await navigateTo(`/brokers/${id}`)
}

onMounted(refresh)
</script>

<template>
  <div class="page-container max-w-7xl">
    <AppPageHeader
      title="Dashboard"
      description="Cluster metrics for brokers, topics, partitions, production, and consumption."
    >
      <AppButton :disabled="loading" @click="refresh">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton :disabled="!brokers?.items?.length" @click="exportCsv">
        <Download class="h-4 w-4" :stroke-width="1.75" />
        Export CSV
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <section class="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <AppMetricCard label="Brokers" :value="String(brokers?.broker_count || 0)" helper="Online nodes" />
      <AppMetricCard
        label="Controller"
        :value="brokers?.controller_id == null ? '—' : String(brokers.controller_id)"
        helper="Active controller id"
      />
      <AppMetricCard label="Topics" :value="String(topicCount)" helper="Including internal topics" />
      <AppMetricCard label="Consumer Groups" :value="String(groupCount)" helper="Registered groups" />
    </section>

    <AppCard>
      <AppResultsHeader label="Brokers" :total="brokers?.items?.length || 0" :loading="loading" :last-updated="lastUpdated" />
      <p class="mb-3 text-sm text-muted-foreground">
        Cluster {{ brokers?.cluster_id || "local" }} · Version {{ brokers?.version || "—" }}
      </p>
      <AppTable>
        <thead>
          <tr>
            <th>Broker ID</th>
            <th>Disk usage</th>
            <th>Host</th>
            <th>Port</th>
            <th>Controller</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="item in brokers?.items || []"
            :key="item.id"
            class="!cursor-pointer hover:bg-accent/60"
            role="link"
            tabindex="0"
            @click="openBroker(item.id)"
            @keydown.enter.prevent="openBroker(item.id)"
          >
            <td>
              <NuxtLink :to="`/brokers/${item.id}`" class="font-semibold text-brand-500 hover:underline" @click.stop>
                {{ item.id }}
              </NuxtLink>
            </td>
            <td class="font-mono text-sm">
              {{ formatBytes(item.segment_size) }}
              <span class="text-muted-foreground"> · {{ item.segment_count ?? 0 }} segment(s)</span>
            </td>
            <td class="font-mono">{{ item.host }}</td>
            <td>{{ item.port }}</td>
            <td>
              <span v-if="item.controller" class="badge-success">Controller</span>
              <span v-else class="text-muted-foreground">—</span>
            </td>
          </tr>
          <tr v-if="!loading && !(brokers?.items?.length)">
            <td colspan="5"><AppStateBlock kind="empty" message="No brokers reported." /></td>
          </tr>
        </tbody>
      </AppTable>
    </AppCard>
  </div>
</template>
