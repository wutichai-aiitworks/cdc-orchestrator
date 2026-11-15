<script setup lang="ts">
// SSH tunnels (ADR 0020): for sites that expose only SSH — often a Cloudflare Zero Trust SSH app.
// The cdc-sshtunnel gateway keeps one SSH session per tunnel and listens on stable forward ports
// (cdc-sshtunnel:22xxx). Pipelines and Copy & Sync connections pick a tunnel under
// "Connection method" / "Connect through".
import { CheckCircle2, KeyRound, Loader2, Pencil, Plus, RefreshCw, RotateCw, ShieldCheck, Trash2, XCircle } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import type { SshTestResult, SshTunnel } from "~/composables/useSshTunnels"

definePageMeta({ layout: "default" })

const toast = useToast()
const api = useSshTunnels()
const loading = ref(true)
const error = ref("")
const data = computed(() => api.tunnels.value)
const list = computed(() => data.value?.tunnels || [])
const REDACTED = "********"

const load = async () => {
  try {
    await api.fetchTunnels(true)
    error.value = ""
  } catch (e: any) {
    error.value = formatApiError(e, "Could not load SSH tunnels")
  } finally {
    loading.value = false
  }
}
let timer: ReturnType<typeof setInterval> | null = null
onMounted(() => {
  load()
  timer = setInterval(() => document.visibilityState === "visible" && !editing.value && api.fetchTunnels(true), 10_000)
})
onBeforeUnmount(() => timer && clearInterval(timer))

// ── editor ────────────────────────────────────────────────────────────────
type Draft = {
  name: string
  host: string
  port: string
  username: string
  auth_method: "key" | "password"
  private_key: string
  passphrase: string
  password: string
  host_key_fingerprint: string
  proxy_kind: "none" | "cloudflare_access"
  proxy_hostname: string
  service_token_id: string
  service_token_secret: string
  notes: string
  // test only: database address as seen from the SSH server
  test_host: string
  test_port: string
}
const emptyDraft = (): Draft => ({
  name: "",
  host: "",
  port: "22",
  username: "",
  auth_method: "key",
  private_key: "",
  passphrase: "",
  password: "",
  host_key_fingerprint: "",
  proxy_kind: "cloudflare_access",
  proxy_hostname: "",
  service_token_id: "",
  service_token_secret: "",
  notes: "",
  test_host: "127.0.0.1",
  test_port: "5432",
})
const editing = ref<null | { id?: string; draft: Draft }>(null)
const saving = ref(false)
const testing = ref(false)
const testResult = ref<SshTestResult | null>(null)
const isNew = computed(() => !editing.value?.id)
// Validation messages appear only after Save / Test was tried, not on an empty new form.
const showProblems = ref(false)
const PROXY_OPTIONS = [
  { value: "cloudflare_access" as const, label: "Cloudflare Access" },
  { value: "none" as const, label: "Direct SSH" },
]

const startNew = () => {
  editing.value = { draft: emptyDraft() }
  showProblems.value = false
  testResult.value = null
}
const startEdit = (t: SshTunnel) => {
  const f = t.forwards?.[0]
  editing.value = {
    id: t.id,
    draft: {
      name: t.name,
      host: t.host,
      port: String(t.port || 22),
      username: t.username,
      auth_method: t.auth_method === "password" ? "password" : "key",
      private_key: t.private_key,
      passphrase: t.passphrase,
      password: t.password,
      host_key_fingerprint: t.host_key_fingerprint,
      proxy_kind: t.proxy?.kind === "cloudflare_access" ? "cloudflare_access" : "none",
      proxy_hostname: t.proxy?.hostname || "",
      service_token_id: t.proxy?.service_token_id || "",
      service_token_secret: t.proxy?.service_token_secret || "",
      notes: t.notes,
      test_host: f?.remote_host || "127.0.0.1",
      test_port: String(f?.remote_port || 5432),
    },
  }
  showProblems.value = false
  testResult.value = null
}

const toRequest = (d: Draft) => ({
  name: d.name.trim(),
  host: d.proxy_kind === "cloudflare_access" ? d.host.trim() || d.proxy_hostname.trim() : d.host.trim(),
  port: Number(d.port) || 22,
  username: d.username.trim(),
  auth_method: d.auth_method,
  private_key: d.auth_method === "key" ? d.private_key : "",
  passphrase: d.auth_method === "key" ? d.passphrase : "",
  password: d.auth_method === "password" ? d.password : "",
  host_key_fingerprint: d.host_key_fingerprint.trim(),
  proxy: {
    kind: d.proxy_kind,
    hostname: d.proxy_kind === "cloudflare_access" ? d.proxy_hostname.trim() || d.host.trim() : "",
    service_token_id: d.proxy_kind === "cloudflare_access" ? d.service_token_id.trim() : "",
    service_token_secret: d.proxy_kind === "cloudflare_access" ? d.service_token_secret : "",
  },
  notes: d.notes,
})

const problems = computed(() => {
  const d = editing.value?.draft
  if (!d) return []
  const p: string[] = []
  if (!d.name.trim()) p.push("Name is required.")
  if (d.proxy_kind === "cloudflare_access" ? !(d.proxy_hostname.trim() || d.host.trim()) : !d.host.trim()) p.push("SSH host is required.")
  if (!d.username.trim()) p.push("SSH user is required.")
  if (d.auth_method === "key" && !d.private_key.trim()) p.push("Private key is required.")
  if (d.auth_method === "password" && !d.password) p.push("Password is required.")
  if (d.proxy_kind === "cloudflare_access" && !!d.service_token_id.trim() !== !!d.service_token_secret) p.push("Give both the Cloudflare service token ID and secret.")
  return p
})

const runTest = async (withDb: boolean) => {
  if (!editing.value) return
  const d = editing.value.draft
  testing.value = true
  testResult.value = null
  try {
    testResult.value = await api.testTunnel(toRequest(d), editing.value.id, withDb ? d.test_host.trim() : "", withDb ? Number(d.test_port) : null)
  } catch (e) {
    testResult.value = { ok: false, error: formatApiError(e, "Test failed") }
  } finally {
    testing.value = false
  }
}
const tryTest = (withDb: boolean) => {
  if (problems.value.length) {
    showProblems.value = true
    return
  }
  runTest(withDb)
}
const pinObserved = () => {
  if (editing.value && testResult.value?.host_key_fingerprint) {
    editing.value.draft.host_key_fingerprint = testResult.value.host_key_fingerprint
    testResult.value = null
    runTest(false)
  }
}

const save = async () => {
  if (!editing.value) return
  if (problems.value.length) {
    showProblems.value = true
    return
  }
  saving.value = true
  try {
    const body = toRequest(editing.value.draft)
    const res = editing.value.id ? await api.updateTunnel(editing.value.id, body) : await api.createTunnel(body)
    toast.success(isNew.value ? "SSH tunnel created" : "SSH tunnel saved")
    if (res.gateway_error) toast.error("Saved. The tunnel starts when the gateway is available.")
    if (!res.host_key_fingerprint) toast.error("Saved. Pin the host key to start the tunnel.")
    editing.value = null
  } catch (e) {
    toast.error(formatApiError(e, "Could not save the SSH tunnel"))
  } finally {
    saving.value = false
  }
}

// ── list ──────────────────────────────────────────────────────────────────
const deleting = ref<SshTunnel | null>(null)
const confirmDelete = async () => {
  const t = deleting.value
  deleting.value = null
  if (!t) return
  try {
    await api.deleteTunnel(t.id)
    toast.success(`Deleted ${t.name}`)
  } catch (e) {
    toast.error(formatApiError(e, "Could not delete the SSH tunnel"))
  }
}
const reconnect = async (t: SshTunnel) => {
  try {
    await api.reconnect(t.id)
    toast.success(`Reconnecting ${t.name}`)
    setTimeout(() => api.fetchTunnels(true), 2000)
  } catch (e) {
    toast.error(formatApiError(e, "Could not reconnect"))
  }
}

const state = (t: SshTunnel): { tone: string; text: string; detail?: string } => {
  if (!t.host_key_fingerprint) return { tone: "warn", text: "Setup incomplete", detail: "Pin the server's host key to start this tunnel: Edit → Test SSH → Pin." }
  if (!data.value?.gateway_reachable) return { tone: "muted", text: "Status unavailable" }
  if (!t.status) return { tone: "warn", text: "Starting…" }
  if (t.status.connected) return { tone: "ok", text: "Connected", detail: t.status.connected_since ? `Since ${new Date(t.status.connected_since).toLocaleString()}` : undefined }
  return { tone: "bad", text: "Not connected", detail: t.status.last_error }
}
const DOT: Record<string, string> = { ok: "bg-emerald-500", warn: "bg-amber-500", bad: "bg-red-500", muted: "bg-muted-foreground/40" }
const endpoint = (t: SshTunnel) => `${t.username}@${t.proxy?.kind === "cloudflare_access" ? `${t.proxy.hostname} (Cloudflare Access)` : `${t.host}:${t.port || 22}`}`
const liveForward = (t: SshTunnel, port: number) => t.status?.forwards?.find((f) => f.listen_port === port)
</script>

<template>
  <div class="page-container">
    <AppPageHeader
      title="SSH tunnels"
      description="Connect to databases that are reachable only over SSH."
    >
      <AppButton title="Refresh" @click="load"><RefreshCw class="h-4 w-4" :stroke-width="1.75" />Refresh</AppButton>
      <AppButton variant="primary" :disabled="!data?.configured" data-testid="ssh-tunnel-new" @click="startNew"><Plus class="h-4 w-4" :stroke-width="2" />New SSH tunnel</AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="loading" kind="loading" message="Loading SSH tunnels…" />
    <AppStateBlock v-else-if="error || data?.error" kind="error" :message="error || data?.error || ''" />
    <div v-else-if="!data?.configured" class="alert-warning">
      SSH tunnels are not enabled on this server. Contact your administrator.
    </div>
    <div v-else-if="!data?.gateway_reachable" class="alert-warning">
      The tunnel gateway is not responding. Your tunnels are kept and resume when it is back.
    </div>

    <!-- Editor -->
    <section v-if="editing" class="stat-card space-y-5">
      <div class="flex items-center justify-between gap-3">
        <h2 class="section-title">{{ isNew ? "New SSH tunnel" : `Edit ${editing.draft.name}` }}</h2>
        <AppButton variant="ghost" size="sm" @click="editing = null">Cancel</AppButton>
      </div>

      <div class="grid gap-x-4 gap-y-4 md:grid-cols-2">
        <label>
          <span class="auth-label">Name</span>
          <input v-model="editing.draft.name" class="auth-input" placeholder="Branch DB (Cloudflare)" />
        </label>
        <div>
          <AppFormLabel label="How SSH is reached">
            <p><b>Cloudflare Access</b>: SSH is published through Cloudflare Zero Trust. The gateway connects through cloudflared with a service token.</p>
            <p class="mt-1"><b>Direct SSH</b>: the SSH server is reachable from this server over the network or VPN.</p>
          </AppFormLabel>
          <TransferSegmented v-model="editing.draft.proxy_kind" label="How SSH is reached" :options="PROXY_OPTIONS" />
        </div>

        <!-- Where SSH is -->
        <template v-if="editing.draft.proxy_kind === 'cloudflare_access'">
          <label class="md:col-span-2">
            <AppFormLabel label="SSH hostname" hint="The hostname of the Cloudflare Access SSH application, as used with cloudflared access ssh --hostname." />
            <input v-model.trim="editing.draft.proxy_hostname" class="auth-input font-mono !text-xs" placeholder="ssh.branch.example.com" spellcheck="false" />
          </label>
          <label>
            <AppFormLabel label="Service token ID" hint="Lets the gateway sign in without a browser. The SSH application needs a Service Auth policy for this token." />
            <input v-model.trim="editing.draft.service_token_id" class="auth-input font-mono !text-xs" placeholder="xxxxxxxx.access" autocomplete="off" spellcheck="false" />
          </label>
          <label>
            <span class="auth-label">Service token secret</span>
            <input v-model="editing.draft.service_token_secret" type="password" class="auth-input font-mono !text-xs" autocomplete="new-password" />
          </label>
        </template>
        <template v-else>
          <label>
            <span class="auth-label">SSH host</span>
            <input v-model.trim="editing.draft.host" class="auth-input font-mono !text-xs" placeholder="bastion.site.example" spellcheck="false" />
          </label>
          <label>
            <span class="auth-label">SSH port</span>
            <input v-model.trim="editing.draft.port" class="auth-input tabular-nums" inputmode="numeric" placeholder="22" />
          </label>
        </template>

        <!-- Login -->
        <label>
          <span class="auth-label">SSH user</span>
          <input v-model.trim="editing.draft.username" class="auth-input" placeholder="cdc-tunnel" autocomplete="off" spellcheck="false" />
        </label>
        <label>
          <span class="auth-label">Authentication</span>
          <select v-model="editing.draft.auth_method" class="auth-input">
            <option value="key">Private key (recommended)</option>
            <option value="password">Password</option>
          </select>
        </label>
        <template v-if="editing.draft.auth_method === 'key'">
          <label class="md:col-span-2">
            <AppFormLabel label="Private key" hint="Use a dedicated key. On the SSH server, limit it to the database with permitopen in authorized_keys." />
            <textarea v-model="editing.draft.private_key" rows="4" class="auth-input font-mono !text-xs" placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" spellcheck="false" />
          </label>
          <label>
            <span class="auth-label">Key passphrase (optional)</span>
            <input v-model="editing.draft.passphrase" type="password" class="auth-input" autocomplete="new-password" />
          </label>
        </template>
        <label v-else>
          <span class="auth-label">Password</span>
          <input v-model="editing.draft.password" type="password" class="auth-input" autocomplete="new-password" />
        </label>
        <label>
          <AppFormLabel label="Host key" hint="Confirms you connect to the right server. Test SSH reads it for you to confirm." />
          <input v-model.trim="editing.draft.host_key_fingerprint" class="auth-input font-mono !text-xs" placeholder="SHA256:…" spellcheck="false" />
        </label>
        <label class="md:col-span-2">
          <span class="auth-label">Notes</span>
          <input v-model="editing.draft.notes" class="auth-input" placeholder="Optional" />
        </label>
      </div>

      <!-- Test -->
      <div class="space-y-3 rounded-xl border border-border bg-muted/30 p-4">
        <div class="grid items-end gap-3 sm:grid-cols-[minmax(0,1fr)_7rem_auto_auto]">
          <label>
            <AppFormLabel label="Database host" hint="As seen from the SSH server, often 127.0.0.1." />
            <input v-model.trim="editing.draft.test_host" class="auth-input font-mono !text-xs" placeholder="127.0.0.1" />
          </label>
          <label>
            <span class="auth-label">Port</span>
            <input v-model.trim="editing.draft.test_port" class="auth-input tabular-nums" inputmode="numeric" placeholder="5432" />
          </label>
          <AppButton :disabled="testing" data-testid="ssh-tunnel-test" @click="tryTest(false)">
            <Loader2 v-if="testing" class="h-4 w-4 animate-spin" :stroke-width="2" />
            <ShieldCheck v-else class="h-4 w-4" :stroke-width="1.75" />Test SSH
          </AppButton>
          <AppButton :disabled="testing || !editing.draft.host_key_fingerprint" :title="editing.draft.host_key_fingerprint ? '' : 'Pin the host key first (Test SSH)'" @click="tryTest(true)">
            <KeyRound class="h-4 w-4" :stroke-width="1.75" />Test database
          </AppButton>
        </div>
        <div v-if="testResult" class="text-xs" :class="testResult.ok ? 'text-emerald-700 dark:text-emerald-400' : 'text-red-600 dark:text-red-400'">
          <p v-if="testResult.ok" class="flex items-center gap-1.5">
            <CheckCircle2 class="h-3.5 w-3.5" :stroke-width="2" />{{ testResult.target ? "Connected to the database" : "SSH connection OK" }}
          </p>
          <template v-else-if="testResult.needs_host_key">
            <p class="flex flex-wrap items-center gap-1 text-foreground">
              Confirm the server's host key: <code class="font-mono">{{ testResult.host_key_fingerprint }}</code>
              <AppInfoHint text="Check that it matches the server's key before trusting it. Nothing has been sent to the server yet." />
            </p>
            <AppButton size="sm" variant="primary" class="mt-2" @click="pinObserved">Trust and test again</AppButton>
          </template>
          <p v-else class="flex items-center gap-1.5">
            <XCircle class="h-3.5 w-3.5" :stroke-width="2" />
            {{ testResult.host_key_mismatch ? "Host key does not match" : testResult.ssh_ok ? "SSH OK, database not reachable" : "Could not connect" }}
            <AppInfoHint aria-label="Details">
              <p>{{ testResult.error }}</p>
              <p v-if="testResult.hint" class="mt-1">{{ testResult.hint }}</p>
            </AppInfoHint>
          </p>
        </div>
      </div>

      <ul v-if="showProblems && problems.length" class="list-disc space-y-0.5 pl-5 text-xs text-amber-600 dark:text-amber-400">
        <li v-for="p in problems" :key="p">{{ p }}</li>
      </ul>
      <div class="flex justify-end gap-2">
        <AppButton variant="ghost" @click="editing = null">Cancel</AppButton>
        <AppButton variant="primary" :disabled="saving" @click="save">{{ isNew ? "Create tunnel" : "Save" }}</AppButton>
      </div>
    </section>

    <!-- List -->
    <section v-if="data?.configured" class="space-y-3">
      <div v-if="!list.length && !editing" class="rounded-xl border border-dashed border-border p-8 text-center">
        <KeyRound class="mx-auto h-8 w-8 text-muted-foreground" :stroke-width="1.5" />
        <h2 class="mt-3 text-base font-semibold text-foreground">No SSH tunnels yet</h2>
        <p class="mx-auto mt-1 max-w-md text-sm text-muted-foreground">
          Create one per SSH server, then choose it in a pipeline.
        </p>
        <AppButton variant="primary" class="mt-4" @click="startNew"><Plus class="h-4 w-4" :stroke-width="2" />New SSH tunnel</AppButton>
      </div>

      <article v-for="t in list" :key="t.id" class="stat-card">
        <div class="flex flex-wrap items-start justify-between gap-3">
          <div class="min-w-0">
            <div class="flex items-center gap-2">
              <span class="h-2 w-2 rounded-full" :class="DOT[state(t).tone]" aria-hidden="true" />
              <h3 class="truncate text-base font-semibold text-foreground">{{ t.name }}</h3>
            </div>
            <p class="mt-0.5 inline-flex items-center gap-1 text-sm text-muted-foreground">
              {{ state(t).text }}
              <AppInfoHint v-if="state(t).detail" aria-label="Details">
                <p>{{ state(t).detail }}</p>
              </AppInfoHint>
            </p>
          </div>
          <div class="flex flex-wrap gap-1.5">
            <AppButton v-if="t.status && !t.status.connected" size="sm" @click="reconnect(t)"><RotateCw class="h-3.5 w-3.5" :stroke-width="1.75" />Retry now</AppButton>
            <AppButton size="sm" variant="ghost" @click="startEdit(t)"><Pencil class="h-3.5 w-3.5" :stroke-width="1.75" />Edit</AppButton>
            <AppButton size="sm" variant="ghost" title="Delete" @click="deleting = t"><Trash2 class="h-3.5 w-3.5" :stroke-width="1.75" /></AppButton>
          </div>
        </div>
        <dl class="mt-3 grid gap-x-6 gap-y-2 text-sm sm:grid-cols-2 lg:grid-cols-3">
          <div><dt class="text-xs text-muted-foreground">SSH</dt><dd class="truncate font-mono text-xs text-foreground">{{ endpoint(t) }}</dd></div>
          <div><dt class="text-xs text-muted-foreground">Host key</dt><dd class="truncate font-mono text-xs text-foreground">{{ t.host_key_fingerprint || "not pinned" }}</dd></div>
          <div><dt class="text-xs text-muted-foreground">Used by</dt><dd class="truncate text-xs text-foreground">{{ t.used_by?.length ? t.used_by.join(", ") : "—" }}</dd></div>
        </dl>
        <div v-if="t.forwards?.length" class="mt-3 overflow-x-auto">
          <table class="w-full text-xs">
            <thead class="text-muted-foreground">
              <tr><th class="py-1 text-left font-medium">Tunnel address</th><th class="py-1 text-left font-medium">Database</th><th class="py-1 text-left font-medium">State</th></tr>
            </thead>
            <tbody>
              <tr v-for="f in t.forwards" :key="f.listen_port" class="border-t border-border">
                <td class="py-1 font-mono">{{ t.forward_host || "cdc-sshtunnel" }}:{{ f.listen_port }}</td>
                <td class="py-1 font-mono">{{ f.remote_host }}:{{ f.remote_port }}</td>
                <td class="py-1">
                  <template v-if="liveForward(t, f.listen_port)?.listening">Active · {{ liveForward(t, f.listen_port)?.active_connections }}</template>
                  <span v-else class="text-muted-foreground">Inactive</span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </article>
    </section>

    <AppConfirmDialog
      :open="!!deleting"
      :title="`Delete ${deleting?.name}?`"
      confirm-label="Delete SSH tunnel"
      danger
      @update:open="!$event && (deleting = null)"
      @confirm="confirmDelete"
    >
      <p>Connections that use this tunnel will stop working.</p>
    </AppConfirmDialog>
  </div>
</template>
