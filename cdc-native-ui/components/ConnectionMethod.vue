<script setup lang="ts">
// Pipeline source/target "Connection method" (ADR 0020): Direct, or through an SSH tunnel for
// sites that expose only SSH (e.g. a Cloudflare Zero Trust SSH app). With a tunnel the host and
// port fields mean the database address as seen from the SSH server; the orchestrator gives the
// pipeline a stable cdc-sshtunnel:<port> forward when it is saved or tested.
import { CheckCircle2, Loader2, PlugZap, XCircle } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"

const model = defineModel<string>({ default: "" })
const props = defineProps<{ host?: string; port?: string; side?: "source" | "target" }>()

const { tunnels, fetchTunnels, probe } = useSshTunnels()
onMounted(() => fetchTunnels())

const chosenSsh = ref(false)
const METHOD_OPTIONS = [
  { value: "direct" as const, label: "Direct" },
  { value: "ssh" as const, label: "SSH tunnel" },
]
const mode = computed<"direct" | "ssh">(() => (model.value || chosenSsh.value ? "ssh" : "direct"))
const list = computed(() => tunnels.value?.tunnels || [])
const current = computed(() => list.value.find((t) => t.id === model.value))
const missing = computed(() => !!model.value && !!tunnels.value && !current.value)
const forward = computed(() => {
  const t = current.value
  if (!t || !props.host) return null
  const port = Number(props.port || 0)
  return t.forwards?.find((f) => f.remote_host.toLowerCase() === props.host!.trim().toLowerCase() && f.remote_port === port) || null
})
const via = computed(() => `${current.value?.forward_host || "cdc-sshtunnel"}:${forward.value ? forward.value.listen_port : "…"}`)

const setMode = (m: "direct" | "ssh") => {
  chosenSsh.value = m === "ssh"
  if (m === "direct") model.value = ""
  else if (!model.value && list.value.length === 1) model.value = list.value[0]!.id
  result.value = null
}

const checking = ref(false)
const result = ref<{ ok: boolean; text: string; hint?: string } | null>(null)
watch(() => `${model.value}|${props.host}|${props.port}`, () => (result.value = null))
const test = async () => {
  if (!model.value || !props.host || !props.port) return
  checking.value = true
  result.value = null
  try {
    const r = await probe(model.value, props.host.trim(), Number(props.port))
    result.value = r.ok
      ? { ok: true, text: `Database reachable${r.ms != null ? ` (${r.ms} ms)` : ""}` }
      : { ok: false, text: "Database not reachable through the tunnel", hint: [r.error, r.hint].filter(Boolean).join(" — ") }
  } catch (e) {
    result.value = { ok: false, text: "Test failed", hint: formatApiError(e, "Test failed") }
  } finally {
    checking.value = false
  }
}
</script>

<template>
  <div class="space-y-3 rounded-xl border border-border bg-muted/30 p-3" data-testid="connection-method">
    <div>
      <AppFormLabel label="Connection method">
        <p><b>Direct</b>: the pipeline connects to the database host itself.</p>
        <p class="mt-1"><b>SSH tunnel</b>: for databases reachable only over SSH, for example through Cloudflare Zero Trust.</p>
      </AppFormLabel>
      <TransferSegmented :model-value="mode" label="Connection method" :options="METHOD_OPTIONS" data-testid="connection-method-switch" @update:model-value="setMode" />
    </div>

    <template v-if="mode === 'ssh'">
      <p v-if="tunnels?.error && !list.length" class="text-xs text-red-600 dark:text-red-400">
        Could not load SSH tunnels. <button type="button" class="text-primary hover:underline" @click="fetchTunnels(true)">Retry</button>
      </p>
      <p v-else-if="tunnels && !tunnels.configured" class="text-xs text-amber-600 dark:text-amber-400">
        SSH tunnels are not enabled on this server. Contact your administrator.
      </p>
      <p v-else-if="!list.length && !model" class="text-xs text-muted-foreground">
        No SSH tunnels yet. <NuxtLink to="/ssh-tunnels" class="text-primary hover:underline">Create one</NuxtLink>
      </p>
      <select v-else class="auth-input" :value="model" aria-label="SSH tunnel" data-testid="connection-method-tunnel" @change="model = ($event.target as HTMLSelectElement).value">
        <option value="" disabled>Choose an SSH tunnel…</option>
        <option v-for="t in list" :key="t.id" :value="t.id">
          {{ t.name }}{{ t.status ? (t.status.connected ? " · connected" : " · not connected") : "" }}
        </option>
        <option v-if="missing" :value="model">Deleted tunnel</option>
      </select>

      <div v-if="current" class="flex flex-wrap items-center gap-2 text-xs">
        <span
          class="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 font-medium"
          :class="current.status?.connected ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-400' : 'bg-amber-500/10 text-amber-700 dark:text-amber-400'"
          :title="current.status?.last_error || undefined"
        >
          {{ current.status?.connected ? "Connected" : "Not connected" }}
        </span>
        <span class="inline-flex items-center gap-1 text-muted-foreground">
          via <code>{{ via }}</code>
          <AppInfoHint aria-label="About the tunnel address">
            <p>Enter the {{ side === "target" ? "target" : "database" }} host and port <b>as seen from the SSH server</b>, often <code>127.0.0.1</code>.</p>
            <p class="mt-1">When the pipeline is saved, it gets a fixed tunnel address like this one.</p>
          </AppInfoHint>
        </span>
        <AppButton size="sm" variant="ghost" :disabled="checking || !host || !port || !current.status?.connected" @click="test">
          <Loader2 v-if="checking" class="h-3.5 w-3.5 animate-spin" :stroke-width="2" />
          <PlugZap v-else class="h-3.5 w-3.5" :stroke-width="1.75" />Test
        </AppButton>
        <NuxtLink to="/ssh-tunnels" class="text-primary hover:underline">Manage</NuxtLink>
      </div>
      <div v-if="result" class="flex items-center gap-1.5 text-xs" :class="result.ok ? 'text-emerald-700 dark:text-emerald-400' : 'text-red-600 dark:text-red-400'">
        <component :is="result.ok ? CheckCircle2 : XCircle" class="h-3.5 w-3.5 shrink-0" :stroke-width="2" />
        {{ result.text }}
        <AppInfoHint v-if="result.hint" aria-label="Details">
          <p>{{ result.hint }}</p>
        </AppInfoHint>
      </div>
      <p v-else-if="missing" class="text-xs text-red-600 dark:text-red-400">This tunnel was deleted. Choose another or Direct.</p>
    </template>
  </div>
</template>
