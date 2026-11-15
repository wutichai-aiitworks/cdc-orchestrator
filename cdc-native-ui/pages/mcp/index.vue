<script setup lang="ts">
// MCP overview: is mcp-cdc up, how is it configured, how do chat clients connect, how is access
// managed. Read-only: keys live in the server environment (MCP_CLIENT_KEYS), never in the UI.
import { Copy, RefreshCw } from "@lucide/vue"
import { MCP_DAILY_CHECK } from "~/utils/mcpTools"

definePageMeta({ layout: "default" })

const toast = useToast()
const { info, fetchInfo, endpointUrl } = useMcp()
const loading = ref(true)
const load = async () => {
  loading.value = true
  await fetchInfo(true)
  loading.value = false
}
onMounted(load)

const tools = computed(() => info.value?.tools || [])
const readCount = computed(() => tools.value.filter((t) => t.read_only).length)

type Client = "claude-code" | "desktop" | "cursor" | "chatgpt"
const client = ref<Client>("claude-code")
const CLIENTS = [
  { value: "claude-code" as const, label: "Claude Code" },
  { value: "desktop" as const, label: "claude.ai / Desktop" },
  { value: "chatgpt" as const, label: "ChatGPT" },
  { value: "cursor" as const, label: "Cursor / VS Code" },
]
const snippet = computed(() => {
  const url = endpointUrl.value
  switch (client.value) {
    case "claude-code":
      return `claude mcp add --transport http cdc ${url} \\\n  --header "Authorization: Bearer <your MCP key>"`
    case "cursor":
      return JSON.stringify({ mcpServers: { cdc: { url, headers: { Authorization: "Bearer <your MCP key>" } } } }, null, 2)
    case "desktop":
      return `Settings → Connectors → Add custom connector\nURL: ${url}\nConnect → paste your MCP key on the approval page`
    case "chatgpt":
      return `Settings → Apps & Connectors → Advanced → Developer mode\nCreate connector → URL: ${url} → OAuth\nApprove with your MCP key`
  }
  return ""
})
const needsOauth = computed(() => client.value === "desktop" || client.value === "chatgpt")

const copy = async (text: string, what = "Copied") => {
  try {
    await navigator.clipboard.writeText(text)
    toast.success(what)
  } catch {
    toast.error("Copy failed")
  }
}
</script>

<template>
  <div class="page-container">
    <AppPageHeader title="MCP" description="Operate and monitor CDC from chat clients such as Claude, ChatGPT and Cursor.">
      <AppButton title="Refresh" @click="load"><RefreshCw class="h-4 w-4" :stroke-width="1.75" />Refresh</AppButton>
      <AppButton as="link" to="/mcp/tools">View tools</AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="loading && !info" kind="loading" message="Loading MCP status…" />
    <div v-else-if="info && !info.reachable" class="alert-warning">
      {{ info.error || "mcp-cdc is not reachable." }}
      <AppInfoHint aria-label="How to start mcp-cdc">
        <p>mcp-cdc is optional. Start it with the <code>agent</code> compose profile, or set <code>MCP_URL</code> on the UI if it runs elsewhere.</p>
      </AppInfoHint>
    </div>

    <template v-if="info?.reachable">
      <!-- Status -->
      <section class="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <AppMetricCard label="Service" :value="`mcp-cdc ${info.version || ''}`" helper="Online" tone="success" />
        <AppMetricCard label="Tools" :value="tools.length" :helper="`${readCount} read-only · ${tools.length - readCount} write`" />
        <AppMetricCard
          label="Write tools"
          :value="info.write_tools_enabled ? 'Enabled' : 'Disabled'"
          helper="ALLOW_WRITE_TOOLS; also needs a write-scoped key"
          :tone="info.write_tools_enabled ? 'warning' : 'neutral'"
        />
        <AppMetricCard label="Client keys" :value="`${info.clients?.read ?? 0} read · ${info.clients?.write ?? 0} write`" helper="From MCP_CLIENT_KEYS" />
        <AppMetricCard label="Active sessions" :value="`${info.active_sessions ?? 0} / ${info.max_sessions ?? 0}`" :helper="`Idle timeout ${Math.round((info.session_ttl_secs ?? 0) / 60)} min`" />
        <AppMetricCard
          label="Rate limit"
          :value="info.rate_limit?.enabled ? `${info.rate_limit.requests_per_minute}/min` : 'Off'"
          helper="Per client"
        />
        <AppMetricCard label="OAuth" :value="info.oauth?.enabled ? 'On' : 'Off'" helper="For claude.ai and ChatGPT connectors" />
        <AppMetricCard label="Public URL" :value="info.oauth?.public_url ? 'Set' : 'Not set'" :helper="info.oauth?.public_url || 'Local only (MCP_PUBLIC_URL)'" :tone="info.oauth?.public_url ? 'neutral' : 'info'" />
      </section>

      <!-- Connect -->
      <section class="stat-card space-y-4">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <h2 class="section-title flex items-center gap-1.5">
            Connect a chat client
            <AppInfoHint aria-label="About connecting">
              <p>Every client uses the same endpoint. Give each person or tool its own key, so access can be revoked and audited per client.</p>
              <p class="mt-1">claude.ai and ChatGPT connect from the internet: they need an https public URL (MCP_PUBLIC_URL) and OAuth.</p>
            </AppInfoHint>
          </h2>
          <TransferSegmented v-model="client" label="Client" :options="CLIENTS" />
        </div>
        <div>
          <span class="auth-label">Endpoint</span>
          <div class="flex gap-2">
            <code class="auth-input flex items-center overflow-x-auto font-mono !text-xs">{{ endpointUrl }}</code>
            <AppButton title="Copy endpoint" @click="copy(endpointUrl, 'Endpoint copied')"><Copy class="h-4 w-4" :stroke-width="1.75" /></AppButton>
          </div>
        </div>
        <div class="relative">
          <pre class="overflow-x-auto rounded-lg border border-border bg-muted/40 p-3 font-mono text-xs leading-relaxed">{{ snippet }}</pre>
          <AppButton size="sm" variant="ghost" class="absolute right-2 top-2" @click="copy(snippet)"><Copy class="h-3.5 w-3.5" :stroke-width="1.75" />Copy</AppButton>
        </div>
        <p v-if="needsOauth && (!info.oauth?.enabled || !info.oauth?.public_url)" class="text-xs text-amber-600 dark:text-amber-400">
          This client needs OAuth and a public https URL. {{ !info.oauth?.enabled ? "OAuth is off." : "MCP_PUBLIC_URL is not set." }}
        </p>
      </section>

      <!-- Access -->
      <section class="stat-card space-y-3">
        <h2 class="section-title flex items-center gap-1.5">
          Manage access
          <AppInfoHint aria-label="About keys">
            <p>Keys are configured on the server, not in this UI, and are never shown here. Changes need an mcp-cdc restart.</p>
          </AppInfoHint>
        </h2>
        <div class="grid gap-3 md:grid-cols-3">
          <div class="rounded-lg border border-border p-3">
            <p class="text-sm font-medium text-foreground">Add or revoke a key</p>
            <p class="mt-1 text-xs text-muted-foreground">Edit <code>MCP_CLIENT_KEYS</code> (production: <code>secrets/mcp_client_keys</code>) and restart mcp-cdc.</p>
            <pre class="mt-2 overflow-x-auto rounded bg-muted/40 p-2 font-mono text-[11px]">name:read:&lt;key&gt;,ops:write:&lt;key&gt;</pre>
            <p class="mt-1 text-xs text-muted-foreground">New key: <code>openssl rand -hex 24</code></p>
          </div>
          <div class="rounded-lg border border-border p-3">
            <p class="text-sm font-medium text-foreground">Read vs write</p>
            <p class="mt-1 text-xs text-muted-foreground">
              A <b>read</b> key can use every monitoring and status tool. Write tools also need a <b>write</b> key and
              <code>ALLOW_WRITE_TOOLS=true</code>.
            </p>
            <p class="mt-1 text-xs text-muted-foreground">Recommended: read keys for chat users, write keys for operators only.</p>
          </div>
          <div class="rounded-lg border border-border p-3">
            <p class="text-sm font-medium text-foreground">Audit</p>
            <p class="mt-1 text-xs text-muted-foreground">Actions through MCP are recorded as <code>mcp:&lt;client name&gt;</code>.</p>
            <NuxtLink to="/mcp/activity" class="mt-1 inline-block text-xs text-primary hover:underline">View MCP activity</NuxtLink>
          </div>
        </div>
      </section>

      <!-- Daily check -->
      <section class="stat-card space-y-2">
        <h2 class="section-title flex items-center gap-1.5">
          Daily production check
          <AppInfoHint text="Paste into the chat each morning during a rollout. It only reads." />
        </h2>
        <div class="flex items-start gap-2">
          <p class="flex-1 rounded-lg border border-border bg-muted/40 p-3 text-sm">{{ MCP_DAILY_CHECK }}</p>
          <AppButton title="Copy prompt" @click="copy(MCP_DAILY_CHECK, 'Prompt copied')"><Copy class="h-4 w-4" :stroke-width="1.75" /></AppButton>
        </div>
      </section>
    </template>
  </div>
</template>
