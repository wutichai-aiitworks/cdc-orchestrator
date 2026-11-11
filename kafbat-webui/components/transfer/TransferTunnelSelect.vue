<script setup lang="ts">
// "Connect through": direct, or a WireGuard tunnel to the site (ADR 0010). With a tunnel, the host
// and port are the database's address as seen from the site; the orchestrator swaps them for a
// cdc-wireguard forward when a run or test starts. Hidden until at least one tunnel exists.
const model = defineModel<string | null | undefined>({ default: null })
const { tunnels, fetchTunnels } = useWireguard()
onMounted(() => fetchTunnels())
const list = computed(() => tunnels.value?.tunnels || [])
const current = computed(() => list.value.find((t) => t.id === model.value))
const missing = computed(() => !!model.value && !!tunnels.value && !current.value)
</script>

<template>
  <div v-if="list.length || model" class="space-y-1">
    <label class="block">
      <span class="auth-label !mb-1.5">Connect through</span>
      <select class="auth-input" :value="model || ''" @change="model = ($event.target as HTMLSelectElement).value || null">
        <option value="">Direct (the tool reaches the host itself)</option>
        <option v-for="t in list" :key="t.id" :value="t.id">
          WireGuard: {{ t.name }}{{ t.status ? (t.status.connected ? " · connected" : " · no handshake") : "" }}
        </option>
        <option v-if="missing" :value="model!">Deleted tunnel</option>
      </select>
    </label>
    <p v-if="current" class="text-xs text-muted-foreground">
      Enter the host and port <span class="font-medium text-foreground">as seen from the site</span> (for example its LAN address or
      {{ current.site_address?.split("/")[0] || "the site's tunnel address" }}). Runs and tests go through the cdc-wireguard gateway.
      <NuxtLink to="/tunnels" class="text-primary hover:underline">Manage tunnels</NuxtLink>
    </p>
    <p v-else-if="missing" class="text-xs text-red-600 dark:text-red-400">This tunnel was deleted; choose another or Direct.</p>
  </div>
</template>
