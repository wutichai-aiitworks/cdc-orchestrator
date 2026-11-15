<script setup lang="ts">
// "Connect through": direct, a WireGuard tunnel to the site (ADR 0010), or an SSH tunnel (ADR 0020).
// With a tunnel, host and port are the database's address as seen from the site / SSH server; the
// orchestrator swaps them for a gateway forward when a run or test starts. Hidden until a tunnel exists.
const model = defineModel<string | null | undefined>({ default: null })
const ssh = defineModel<string | null | undefined>("ssh", { default: null })
const { tunnels, fetchTunnels } = useWireguard()
const { tunnels: sshTunnels, fetchTunnels: fetchSsh } = useSshTunnels()
onMounted(() => {
  fetchTunnels()
  fetchSsh()
})
const list = computed(() => tunnels.value?.tunnels || [])
const sshList = computed(() => sshTunnels.value?.tunnels || [])
const current = computed(() => list.value.find((t) => t.id === model.value))
const currentSsh = computed(() => sshList.value.find((t) => t.id === ssh.value))
const missing = computed(() => !!model.value && !!tunnels.value && !current.value)
const missingSsh = computed(() => !!ssh.value && !!sshTunnels.value && !currentSsh.value)
const value = computed(() => (ssh.value ? `ssh:${ssh.value}` : model.value ? `wg:${model.value}` : ""))
const pick = (v: string) => {
  model.value = v.startsWith("wg:") ? v.slice(3) : null
  ssh.value = v.startsWith("ssh:") ? v.slice(4) : null
}
</script>

<template>
  <div v-if="list.length || sshList.length || model || ssh" class="space-y-1">
    <label class="block">
      <span class="auth-label !mb-1.5">
        Connect through
        <AppInfoHint aria-label="About tunnels">
          <p>With a tunnel, enter the host and port <b>as seen from the site or SSH server</b>. Runs and tests go through the tunnel gateway.</p>
          <p class="mt-2"><NuxtLink to="/ssh-tunnels" class="text-primary hover:underline">Manage SSH tunnels</NuxtLink></p>
        </AppInfoHint>
      </span>
      <select class="auth-input" :value="value" @change="pick(($event.target as HTMLSelectElement).value)">
        <option value="">Direct (the tool reaches the host itself)</option>
        <optgroup v-if="sshList.length || missingSsh" label="SSH tunnel">
          <option v-for="t in sshList" :key="t.id" :value="`ssh:${t.id}`">
            SSH: {{ t.name }}{{ t.status ? (t.status.connected ? " · connected" : " · not connected") : "" }}
          </option>
          <option v-if="missingSsh" :value="`ssh:${ssh}`">Deleted SSH tunnel</option>
        </optgroup>
        <optgroup v-if="list.length || missing" label="WireGuard">
          <option v-for="t in list" :key="t.id" :value="`wg:${t.id}`">
            WireGuard: {{ t.name }}{{ t.status ? (t.status.connected ? " · connected" : " · no handshake") : "" }}
          </option>
          <option v-if="missing" :value="`wg:${model}`">Deleted tunnel</option>
        </optgroup>
      </select>
    </label>
    <p v-if="missing || missingSsh" class="text-xs text-red-600 dark:text-red-400">This tunnel was deleted. Choose another or Direct.</p>
  </div>
</template>
