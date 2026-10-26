<script setup lang="ts">
import { Activity, Power, RefreshCw, RotateCw, Server } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"

const { fetchEngineStatus, fetchCluster, runEngineAction } = useControlPlane()
const toast = useToast()
const status = ref<EngineStatus | null>(null)
const cluster = ref<Record<string, any> | null>(null)
const loading = ref(false)
const actionLoading = ref(false)
const error = ref("")
const pendingAction = ref<"up" | "down" | "restart" | null>(null)

const runningServices = computed(
  () => status.value?.services?.filter((service) => service.state.toLowerCase().includes("running")).length || 0
)
const healthyServices = computed(
  () => status.value?.services?.filter((service) => String(service.health || "").toLowerCase() === "healthy").length || 0
)

const refresh = async () => {
  loading.value = true
  error.value = ""
  const results = await Promise.allSettled([fetchEngineStatus(), fetchCluster()])
  if (results[0].status === "fulfilled") status.value = results[0].value
  if (results[1].status === "fulfilled") cluster.value = results[1].value
  const failure = results.find((result) => result.status === "rejected")
  if (failure?.status === "rejected") error.value = formatApiError(failure.reason, "Failed to load infrastructure")
  loading.value = false
}

const confirmAction = async () => {
  if (!pendingAction.value) return
  actionLoading.value = true
  try {
    const result = await runEngineAction(pendingAction.value)
    toast.success(`${result.message}${result.audit_id ? ` · Audit ${result.audit_id}` : ""}`)
    pendingAction.value = null
    await refresh()
  } catch (e: any) {
    error.value = formatApiError(e, "Infrastructure action failed")
  } finally {
    actionLoading.value = false
  }
}

onMounted(refresh)
</script>

<template>
  <div class="page-container">
    <AppPageHeader
      title="Infrastructure"
      description="Operate the CDC runtime and understand service health without using Docker commands."
    >
      <AppButton :disabled="loading" @click="refresh">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <section class="grid grid-cols-1 gap-4 md:grid-cols-3">
      <AppMetricCard label="Runtime Target" :value="status?.engine_target || 'Unavailable'" helper="Container engine provider" tone="brand">
        <Server class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Running Services"
        :value="`${runningServices} / ${status?.services?.length || 0}`"
        helper="Services currently reported as running"
        :tone="runningServices === (status?.services?.length || -1) ? 'success' : 'warning'"
      >
        <Power class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Healthy Services"
        :value="healthyServices"
        helper="Containers with an explicit healthy signal"
        :tone="healthyServices ? 'success' : 'neutral'"
      >
        <Activity class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
    </section>

    <section class="stat-card">
      <div class="page-toolbar mb-4">
        <div>
          <AppSectionTitle title="Runtime Controls" hint="Actions affect the managed CDC containers on this host." />
        </div>
        <div class="flex flex-wrap gap-2">
          <AppButton variant="primary" :disabled="actionLoading" @click="pendingAction = 'up'">
            <Power class="h-4 w-4" :stroke-width="1.75" />
            Start Runtime
          </AppButton>
          <AppButton :disabled="actionLoading" @click="pendingAction = 'restart'">
            <RotateCw class="h-4 w-4" :stroke-width="1.75" />
            Restart Runtime
          </AppButton>
          <AppButton variant="destructive" :disabled="actionLoading" @click="pendingAction = 'down'">
            <Power class="h-4 w-4" :stroke-width="1.75" />
            Stop Runtime
          </AppButton>
        </div>
      </div>
      <p class="rounded-xl border border-amber-500/25 bg-amber-500/10 px-4 py-3 text-sm text-amber-700 dark:text-amber-400">
        Restarting or stopping the runtime interrupts active capture and delivery. Confirm only during an approved maintenance window.
      </p>
    </section>

    <section class="stat-card">
      <div class="mb-4">
        <AppSectionTitle title="Managed Services" hint="Current state and container health reported by the engine provider." />
      </div>
      <AppEmptyState
        v-if="!loading && !status?.services?.length"
        title="No Service Status Available"
        description="Start the runtime or verify the orchestrator can access its container engine."
      >
        <template #icon><Server class="h-5 w-5" :stroke-width="1.75" /></template>
      </AppEmptyState>
      <AppTable v-else>
        <thead>
          <tr>
            <th>Service</th>
            <th>State</th>
            <th>Health</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="service in status?.services || []" :key="service.name">
            <td class="font-medium">{{ service.name }}</td>
            <td><AppStatusBadge :status="service.state" /></td>
            <td><AppStatusBadge :status="service.health || 'Not Reported'" /></td>
          </tr>
        </tbody>
      </AppTable>
    </section>

    <details class="stat-card group">
      <summary class="flex cursor-pointer list-none items-center gap-1.5">
        <span class="section-title">Connect Cluster Details</span>
        <AppInfoHint text="Raw cluster metadata for advanced troubleshooting." />
      </summary>
      <pre class="code-block max-h-80">{{ JSON.stringify(cluster, null, 2) }}</pre>
    </details>
  </div>

  <Teleport to="body">
    <div v-if="pendingAction" class="modal-overlay" @click.self="pendingAction = null">
      <section class="auth-card w-full max-w-md bg-card p-5" role="alertdialog" aria-modal="true">
        <h2 class="section-title">Confirm Runtime Action</h2>
        <p class="mt-2 text-sm text-muted-foreground">
          {{
            pendingAction === "down"
              ? "Stop every managed CDC service? Active replication will be interrupted."
              : pendingAction === "restart"
                ? "Restart every managed CDC service? Expect a temporary interruption."
                : "Start all managed CDC services?"
          }}
        </p>
        <div class="mt-5 flex justify-end gap-2">
          <AppButton :disabled="actionLoading" @click="pendingAction = null">Cancel</AppButton>
          <AppButton
            :variant="pendingAction === 'down' ? 'destructive' : 'primary'"
            :disabled="actionLoading"
            @click="confirmAction"
          >
            Confirm
          </AppButton>
        </div>
      </section>
    </div>
  </Teleport>
</template>
