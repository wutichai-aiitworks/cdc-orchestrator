<script setup lang="ts">
import { RefreshCw, ScrollText, SlidersHorizontal } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"

const { fetchLogServices, fetchLoggers, updateLogger } = useOperationsM3()
const toast = useToast()
const route = useRoute()
const router = useRouter()

const tab = ref<"logs" | "levels">("logs")
const services = ref<Record<string, { service?: string; container?: string }>>({})
const loggers = ref<Record<string, any>>({})
const loading = ref(false)
const error = ref("")

const serviceNames = computed(() => Object.keys(services.value).sort())
/** The selected service lives in the URL (?service=vector) so a view can be shared or bookmarked. */
const selectedService = computed({
  get: () => {
    const q = String(route.query.service || "")
    return serviceNames.value.includes(q) ? q : serviceNames.value[0] || ""
  },
  set: (name: string) => {
    router.replace({ query: { ...route.query, service: name } })
  },
})

const loggerEntries = computed(() => {
  const source = (loggers.value as any)?.loggers || loggers.value || {}
  return Object.entries(source)
    .map(([name, value]: [string, any]) => ({
      name,
      level: String(value?.level || value || "INFO").toUpperCase(),
    }))
    .sort((a, b) => a.name.localeCompare(b.name))
})

const load = async () => {
  loading.value = true
  error.value = ""
  try {
    const [serviceResponse, loggerResponse] = await Promise.all([fetchLogServices(), fetchLoggers()])
    services.value = serviceResponse || {}
    loggers.value = loggerResponse || {}
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load log services")
  } finally {
    loading.value = false
  }
}

const setLevel = async (logger: string, level: string) => {
  try {
    await updateLogger(logger, level)
    toast.success(`${logger} changed to ${level}.`)
    loggers.value = await fetchLoggers()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to update logger level")
  }
}

onMounted(load)
</script>

<template>
  <div class="page-container">
    <AppPageHeader
      title="Logs"
      description="Live tail of the CDC stack's service logs, with runtime logger controls."
    >
      <AppButton :disabled="loading" title="Reload services and logger levels" @click="load">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <div class="inline-flex rounded-xl border border-border bg-muted p-1">
      <button
        type="button"
        :class="['rounded-lg px-4 py-2 text-sm font-medium', tab === 'logs' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground']"
        @click="tab = 'logs'"
      >
        Service Logs
      </button>
      <button
        type="button"
        :class="['rounded-lg px-4 py-2 text-sm font-medium', tab === 'levels' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground']"
        @click="tab = 'levels'"
      >
        Logger Levels
      </button>
    </div>

    <template v-if="tab === 'logs'">
      <AppEmptyState
        v-if="!serviceNames.length && !loading"
        title="No Log Services"
        description="The orchestrator did not return any managed log sources."
      >
        <template #icon><ScrollText class="h-5 w-5" /></template>
      </AppEmptyState>
      <LogViewer
        v-else-if="selectedService"
        :service="selectedService"
        :container="services[selectedService]?.container"
      >
        <template #services>
          <div class="flex h-8 items-center rounded-lg border border-border bg-muted p-0.5" role="tablist" aria-label="Service">
            <button
              v-for="name in serviceNames"
              :key="name"
              type="button"
              role="tab"
              :aria-selected="selectedService === name"
              :title="services[name]?.container"
              class="h-full rounded-md px-3 text-xs font-medium transition-colors"
              :class="selectedService === name ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'"
              @click="selectedService = name"
            >
              {{ name }}
            </button>
          </div>
        </template>
      </LogViewer>
    </template>

    <section v-else class="stat-card">
      <div class="mb-4">
        <AppSectionTitle title="Kafka Connect Logger Levels" hint="Temporarily increase verbosity for targeted troubleshooting. Avoid DEBUG in normal production operation." />
      </div>
      <AppEmptyState
        v-if="!loggerEntries.length"
        title="No Runtime Loggers"
        description="Kafka Connect did not return logger configuration."
      >
        <template #icon><SlidersHorizontal class="h-5 w-5" /></template>
      </AppEmptyState>
      <AppTable v-else>
        <thead><tr><th>Logger</th><th>Current Level</th><th>Change Level</th></tr></thead>
        <tbody>
          <tr v-for="logger in loggerEntries" :key="logger.name">
            <td class="break-all font-mono">{{ logger.name }}</td>
            <td><AppStatusBadge :status="logger.level" /></td>
            <td>
              <AppSelect
                :model-value="logger.level"
                select-class="w-32"
                :aria-label="`Log level for ${logger.name}`"
                @update:model-value="setLevel(logger.name, String($event))"
              >
                <option value="ERROR">ERROR</option>
                <option value="WARN">WARN</option>
                <option value="INFO">INFO</option>
                <option value="DEBUG">DEBUG</option>
                <option value="TRACE">TRACE</option>
              </AppSelect>
            </td>
          </tr>
        </tbody>
      </AppTable>
    </section>
  </div>
</template>
