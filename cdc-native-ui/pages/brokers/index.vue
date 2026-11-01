<script setup lang="ts">
import { Download, RefreshCw } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import { formatBytes } from "~/utils/bytes"
import { downloadCsv } from "~/utils/csv"

const { fetchBrokers } = useKafkaAdmin()
const loading = ref(false)
const error = ref("")
const lastUpdated = ref("")
const result = ref<Awaited<ReturnType<typeof fetchBrokers>> | null>(null)

const load = async () => {
  loading.value = true
  error.value = ""
  try {
    result.value = await fetchBrokers()
    lastUpdated.value = new Date().toLocaleTimeString()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load brokers")
  } finally {
    loading.value = false
  }
}

const exportCsv = () => {
  downloadCsv(
    "brokers",
    (result.value?.items || []).map((item) => ({
      brokerId: item.id,
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

onMounted(load)
</script>

<template>
  <div class="page-container max-w-7xl">
    <AppPageHeader
      title="Brokers"
      description="Inspect brokers, disk usage, partition leaders, controller status, and cluster configuration."
    >
      <AppButton :disabled="loading" @click="load">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton :disabled="!result?.items?.length" @click="exportCsv">
        <Download class="h-4 w-4" :stroke-width="1.75" />
        Export CSV
      </AppButton>
    </AppPageHeader>

    <section class="grid grid-cols-1 gap-4 sm:grid-cols-3">
      <AppMetricCard label="Broker count" :value="String(result?.broker_count || 0)" />
      <AppMetricCard label="Active controller" :value="result?.controller_id == null ? '—' : String(result.controller_id)" />
      <AppMetricCard label="Version" :value="result?.version || '—'" helper="rpk / broker version" />
    </section>

    <AppStateBlock v-if="error" kind="error" :message="error" />
    <AppStateBlock v-else-if="loading && !result" kind="loading" message="Loading brokers..." />

    <AppCard>
      <AppResultsHeader label="Brokers" :total="result?.items?.length || 0" :loading="loading" :last-updated="lastUpdated" />
      <AppTable>
        <thead>
          <tr>
            <th>Broker ID</th>
            <th>Disk usage</th>
            <th>Port</th>
            <th>Host</th>
            <th>Controller</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="item in result?.items || []"
            :key="item.id"
            class="!cursor-pointer hover:bg-accent/60"
            role="link"
            tabindex="0"
            @click="openBroker(item.id)"
            @keydown.enter.prevent="openBroker(item.id)"
          >
            <td>
              <NuxtLink
                :to="`/brokers/${item.id}`"
                class="font-semibold text-brand-500 hover:underline"
                @click.stop
              >
                {{ item.id }}
              </NuxtLink>
            </td>
            <td class="font-mono text-sm">
              {{ formatBytes(item.segment_size) }}
              <span class="text-muted-foreground"> · {{ item.segment_count ?? 0 }} segment(s)</span>
            </td>
            <td>{{ item.port }}</td>
            <td class="font-mono">{{ item.host }}</td>
            <td>
              <span v-if="item.controller" class="badge-success">Yes</span>
              <span v-else class="text-muted-foreground">No</span>
            </td>
          </tr>
          <tr v-if="!loading && !(result?.items?.length)">
            <td colspan="5"><AppStateBlock kind="empty" message="No brokers are online." /></td>
          </tr>
        </tbody>
      </AppTable>
    </AppCard>
  </div>
</template>
