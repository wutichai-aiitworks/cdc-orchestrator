<script setup lang="ts">
// WireGuard tunnels (ADR 0010): reach source/target sites whose database ports are closed. Each
// site runs a WireGuard server on one open UDP port; the cdc-wireguard gateway dials it. Connections
// choose a tunnel under "Connect through".
import { ClipboardList, Copy, Download, KeyRound, Network, Pencil, Plus, RefreshCw, Trash2 } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import { configHasPlaceholders, draftProblems, emptyDraft, fmtBytes, siteManualSteps, toRequest, tunnelStatus, type TunnelDraft } from "~/utils/wireguardView"
import { buildGraph, forceLayout, toGexf } from "~/utils/tunnelGraph"
import type { SiteConfig, TunnelSaveResult, WireguardTunnel } from "~/composables/useWireguard"

definePageMeta({ layout: "default" })

const toast = useToast()
const api = useWireguard()
const loading = ref(true)
const error = ref("")
const data = computed(() => api.tunnels.value)
const list = computed(() => data.value?.tunnels || [])

const load = async () => {
  try {
    await api.fetchTunnels(true)
    error.value = ""
  } catch (e: any) {
    error.value = formatApiError(e, "Could not load tunnels")
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
const editing = ref<null | { id?: string; draft: TunnelDraft }>(null)
const saving = ref(false)
const isNew = computed(() => !editing.value?.id)
const problems = computed(() => (editing.value ? draftProblems(editing.value.draft, isNew.value) : []))
const showProblems = ref(false)

const startNew = () => {
  editing.value = { draft: { ...emptyDraft(list.value), peer_endpoint: prepEndpoint.value } }
  prepOpen.value = false
  showProblems.value = false
  setup.value = null
}
const startEdit = (t: WireguardTunnel) => {
  editing.value = {
    id: t.id,
    draft: {
      name: t.name,
      peer_endpoint: t.peer_endpoint,
      peer_public_key: t.peer_public_key,
      generate_site_keys: false,
      preshared_key: t.preshared_key,
      generate_preshared_key: false,
      address: t.address,
      site_address: t.site_address,
      allowed_ips: [...t.allowed_ips],
      dns: [...t.dns],
      keepalive: t.keepalive ?? 25,
      mtu: t.mtu ?? null,
      notes: t.notes,
    },
  }
  showProblems.value = false
  setup.value = null
}

const save = async () => {
  if (!editing.value) return
  if (problems.value.length) {
    showProblems.value = true
    return
  }
  saving.value = true
  try {
    const body = toRequest(editing.value.draft, isNew.value)
    const res: TunnelSaveResult = isNew.value ? await api.createTunnel(body) : await api.updateTunnel(editing.value.id!, body)
    toast.success(isNew.value ? "Tunnel created" : "Tunnel saved")
    if (res.gateway_error) toast.error(`Saved, but the gateway did not take it yet: ${res.gateway_error}`)
    editing.value = null
    // A new tunnel: show the site's configuration right away (with secrets shown only now).
    if (res.site) setup.value = { tunnel: res, config: res.site, once: res.once || {} }
    readinessOpen[res.id] = true
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not save the tunnel"))
  } finally {
    saving.value = false
  }
}

// ── site setup ────────────────────────────────────────────────────────────
const setup = ref<null | { tunnel: WireguardTunnel; config: SiteConfig; once: { site_private_key?: string; preshared_key?: string } }>(null)
const openSetup = async (t: WireguardTunnel) => {
  try {
    setup.value = { tunnel: t, config: await api.fetchSiteConfig(t.id), once: {} }
    editing.value = null
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not build the site configuration"))
  }
}
/** The site config with the once-shown secrets filled in (they exist only in this browser tab). */
const setupText = computed(() => {
  if (!setup.value) return ""
  let c = setup.value.config.config
  if (setup.value.once.preshared_key) c = c.replace(/PresharedKey = <[^>]+>/, `PresharedKey = ${setup.value.once.preshared_key}`)
  return c
})
const copy = async (text: string, what: string) => {
  await navigator.clipboard.writeText(text)
  toast.success(`${what} copied`)
}
const saveFile = (text: string, name: string) => {
  const a = document.createElement("a")
  a.href = URL.createObjectURL(new Blob([text], { type: "text/plain" }))
  a.download = name
  a.click()
  URL.revokeObjectURL(a.href)
}
const download = () => saveFile(setupText.value, "wg0.conf")
/** The file is usable at the site only when no secret is a placeholder. */
const setupComplete = computed(() => !!setup.value && !configHasPlaceholders(setupText.value))
// Copying the template (placeholders) is allowed, but say so: the site cannot start WireGuard with it.
const TEMPLATE_WARNING = "This copy still has <placeholders> — WireGuard will not start with it. Click “Generate new site keys” first for a complete wg0.conf."
const copyConf = async () => {
  await copy(setupText.value, "wg0.conf")
  if (!setupComplete.value) toast.warning(TEMPLATE_WARNING)
}
const downloadConf = () => {
  download()
  if (!setupComplete.value) toast.warning(TEMPLATE_WARNING)
}
/** What the site's admin runs by hand; nothing is installed automatically. */
const manualSteps = computed(() => (setup.value ? siteManualSteps(setup.value.config.listen_port) : []))

// The site's private key is not stored: new site keys give a complete file (shown once).
const rotateAsk = ref(false)
const rotatePsk = ref(true)
const rotating = ref(false)
const rotateKeys = async () => {
  if (!setup.value) return
  rotating.value = true
  try {
    const res = await api.rotateSiteKeys(setup.value.tunnel.id, rotatePsk.value)
    if (res.gateway_error) toast.error(`New keys saved, but the gateway did not take them yet: ${res.gateway_error}`)
    else toast.success("New site keys generated")
    setup.value = { tunnel: res, config: res.site!, once: res.once || {} }
    rotateAsk.value = false
    load()
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not generate new site keys"))
  } finally {
    rotating.value = false
  }
}
watch(() => setup.value?.tunnel.id, () => {
  rotateAsk.value = false
  rotatePsk.value = true
})

// ── site preparation (before a tunnel exists) ────────────────────────────
const prepOpen = ref(false)
const prepEndpoint = ref("")

// ── readiness ─────────────────────────────────────────────────────────────
/** Which tunnel's readiness panel is open (a new tunnel opens its own). */
const readinessOpen = reactive<Record<string, boolean>>({})

// ── topology ──────────────────────────────────────────────────────────────
const topoFilter = ref<"all" | "connected" | "down">("all")
/** Mesh = radial (gateway in the centre); Graph = Gephi-style force-directed. */
const topoView = ref<"mesh" | "graph">("graph")
/** Download the topology as GEXF to open in Gephi (colours, sizes and positions included). */
const exportGexf = () => {
  const g = buildGraph(list.value, !!data.value?.gateway_reachable)
  const a = document.createElement("a")
  a.href = URL.createObjectURL(new Blob([toGexf(g, forceLayout(g, 1000, 700))], { type: "application/xml" }))
  a.download = `wireguard-topology-${new Date().toISOString().slice(0, 10)}.gexf`
  a.click()
  URL.revokeObjectURL(a.href)
}
const topoTunnels = computed(() =>
  list.value.filter((t) => topoFilter.value === "all" || (topoFilter.value === "connected") === !!t.status?.connected),
)
const usage = computed(() => ({
  tunnels: list.value.length,
  connected: list.value.filter((t) => t.status?.connected).length,
  databases: list.value.reduce((n, t) => n + (t.status?.forwards?.length || 0), 0),
}))
/** Diagram click: open that tunnel's readiness and bring its card into view. */
const focusTunnel = (id: string) => {
  readinessOpen[id] = true
  nextTick(() => document.getElementById(`tunnel-${id}`)?.scrollIntoView({ behavior: "smooth", block: "start" }))
}

// ── delete ────────────────────────────────────────────────────────────────
const deleting = ref<WireguardTunnel | null>(null)
const confirmDelete = async () => {
  if (!deleting.value) return
  try {
    await api.deleteTunnel(deleting.value.id)
    toast.success("Tunnel deleted")
    if (setup.value?.tunnel.id === deleting.value.id) setup.value = null
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not delete the tunnel"))
  } finally {
    deleting.value = null
  }
}

const DOT: Record<string, string> = { ok: "bg-emerald-500", warn: "bg-amber-500", err: "bg-red-500", run: "bg-blue-500", idle: "bg-zinc-400" }
</script>

<template>
  <div class="page-container">
    <AppPageHeader
      title="WireGuard tunnels"
      description="Reach source and target sites whose database ports are not open to the outside. Each site opens one UDP port for WireGuard; connections choose a tunnel under “Connect through”."
    >
      <AppButton title="Refresh" @click="load"><RefreshCw class="h-4 w-4" :stroke-width="1.75" />Refresh</AppButton>
      <AppButton :disabled="!data?.configured" title="What a site must prepare before a tunnel can be created" @click="prepOpen = !prepOpen; editing = null">
        <ClipboardList class="h-4 w-4" :stroke-width="1.75" />Prepare a site
      </AppButton>
      <AppButton variant="primary" :disabled="!data?.configured" @click="startNew"><Plus class="h-4 w-4" :stroke-width="2" />New tunnel</AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="loading" kind="loading" message="Loading tunnels…" />
    <AppStateBlock v-else-if="error" kind="error" :message="error" />
    <div v-else-if="!data?.configured" class="alert-warning">
      WireGuard is not set up: start the cdc-wireguard container and set ORCH_WIREGUARD_URL on the orchestrator (see docs/wireguard-sites.md).
    </div>
    <div v-else-if="!data?.gateway_reachable" class="alert-warning">
      The cdc-wireguard gateway does not answer. Tunnels are kept and pushed to it as soon as it is back.
    </div>

    <!-- Topology -->
    <section v-if="data?.configured && (list.length || !editing)" class="grid gap-4 lg:grid-cols-[minmax(0,1fr)_15rem]">
      <div class="relative">
        <TunnelGraph v-if="topoView === 'graph'" :tunnels="topoTunnels" :gateway-reachable="!!data?.gateway_reachable" @select="focusTunnel" />
        <TunnelTopology v-else :tunnels="topoTunnels" :gateway-reachable="!!data?.gateway_reachable" @select="focusTunnel" />
        <div class="absolute right-3 top-3 flex items-center gap-2">
          <div class="flex rounded-lg border border-border bg-card p-0.5" role="radiogroup" aria-label="Diagram">
            <button
              v-for="v in ([['graph', 'Graph'], ['mesh', 'Mesh']] as const)"
              :key="v[0]"
              type="button"
              role="radio"
              :aria-checked="topoView === v[0]"
              class="rounded-md px-2.5 py-1 text-xs font-medium transition-colors"
              :class="topoView === v[0] ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground'"
              @click="topoView = v[0]"
            >{{ v[1] }}</button>
          </div>
          <select v-model="topoFilter" class="auth-input !w-auto !py-1 text-xs" aria-label="Show sites">
            <option value="all">All sites</option>
            <option value="connected">Connected</option>
            <option value="down">Not connected</option>
          </select>
          <AppButton size="sm" :disabled="!list.length" title="Download a .gexf file to open in Gephi" @click="exportGexf">
            <Download class="h-3.5 w-3.5" :stroke-width="1.75" />Gephi
          </AppButton>
        </div>
      </div>
      <div class="grid grid-cols-3 gap-3 lg:grid-cols-1 lg:content-start">
        <AppMetricCard label="Sites" :value="usage.tunnels" helper="WireGuard tunnels, one per site" />
        <AppMetricCard label="Connected" :value="`${usage.connected} / ${usage.tunnels}`" helper="Handshake within the last 3 minutes" :tone="usage.connected < usage.tunnels ? 'warning' : 'neutral'" />
        <AppMetricCard label="Databases in use" :value="usage.databases" helper="Database addresses reached through tunnels (gateway forwards)" />
      </div>
    </section>

    <!-- Site preparation on its own (before anyone creates the tunnel) -->
    <section v-if="prepOpen && !editing" class="stat-card space-y-3">
      <div class="flex items-center justify-between gap-3">
        <h2 class="section-title">Prepare a site</h2>
        <AppButton variant="ghost" size="sm" @click="prepOpen = false">Close</AppButton>
      </div>
      <SitePreparation v-model:endpoint="prepEndpoint" />
    </section>

    <!-- Editor -->
    <section v-if="editing" class="stat-card space-y-5">
      <div class="flex items-center justify-between gap-3">
        <h2 class="section-title">{{ isNew ? "New tunnel" : `Edit ${editing.draft.name}` }}</h2>
        <AppButton variant="ghost" size="sm" @click="editing = null">Cancel</AppButton>
      </div>
      <SitePreparation v-if="isNew" v-model:endpoint="editing.draft.peer_endpoint" :site-name="editing.draft.name" />

      <div class="grid gap-4 md:grid-cols-2">
        <label>
          <span class="auth-label">Name</span>
          <input v-model="editing.draft.name" class="auth-input" placeholder="Source site" />
        </label>
        <label>
          <span class="auth-label">Site WireGuard endpoint (UDP)</span>
          <input v-model.trim="editing.draft.peer_endpoint" class="auth-input font-mono !text-xs" placeholder="vpn.source.example:51820" spellcheck="false" />
          <span class="mt-1 block text-xs text-muted-foreground">The site's public address and the UDP port it opens for WireGuard. The gateway dials it.</span>
        </label>
      </div>

      <div class="space-y-2">
        <span class="auth-label !mb-0">Site's public key</span>
        <label v-if="isNew" class="flex items-center gap-2 text-sm">
          <input v-model="editing.draft.generate_site_keys" type="checkbox" class="accent-[var(--primary)]" />
          Generate the site's keys here (its private key is shown once, in the site configuration)
        </label>
        <label v-if="isNew" class="flex items-center gap-2 text-sm">
          <input v-model="editing.draft.generate_preshared_key" type="checkbox" class="accent-[var(--primary)]" />
          Generate a preshared key (extra protection; shown once, inside the site configuration)
        </label>
        <p v-else class="text-xs text-muted-foreground">
          Preshared key: {{ editing.draft.preshared_key ? "in use (kept)" : "none" }}. To {{ editing.draft.preshared_key ? "replace it" : "add one" }}, use
          <strong>Site setup → Generate site keys</strong> — the site then gets one complete new wg0.conf.
        </p>
        <input
          v-if="!isNew || !editing.draft.generate_site_keys"
          v-model.trim="editing.draft.peer_public_key"
          class="auth-input font-mono !text-xs"
          placeholder="Paste the output of `wg pubkey` on the site (44 characters)"
          spellcheck="false"
        />
      </div>

      <div class="grid gap-4 md:grid-cols-2">
        <label>
          <span class="auth-label">This gateway's tunnel address</span>
          <input v-model.trim="editing.draft.address" class="auth-input font-mono !text-xs" />
          <span class="mt-1 block text-xs text-muted-foreground">The site allows this address in its database settings.</span>
        </label>
        <label>
          <span class="auth-label">Site's tunnel address</span>
          <input v-model.trim="editing.draft.site_address" class="auth-input font-mono !text-xs" />
          <span class="mt-1 block text-xs text-muted-foreground">Use it as the database host when the database runs on the WireGuard server itself.</span>
        </label>
      </div>

      <TransferTokens
        v-model="editing.draft.allowed_ips"
        label="Reachable through the tunnel"
        help="The site's tunnel address, plus the network or host of the database when it runs on another machine at the site (e.g. 192.168.10.0/24)."
        placeholder="10.66.1.1/32, 192.168.10.0/24"
      />

      <details class="group rounded-lg border border-border">
        <summary class="cursor-pointer list-none px-3 py-2 text-sm font-medium text-foreground">Advanced</summary>
        <div class="grid gap-4 border-t border-border p-3 md:grid-cols-3">
          <TransferTokens v-model="editing.draft.dns" class="md:col-span-3" label="DNS at the site" help="Only if database hosts are names that resolve at the site. Leave empty to use IP addresses." placeholder="192.168.10.1" />
          <label>
            <span class="auth-label">Keepalive (seconds)</span>
            <input v-model.number="editing.draft.keepalive" type="number" min="0" max="3600" class="auth-input tabular-nums" />
          </label>
          <label>
            <span class="auth-label">MTU</span>
            <input v-model.number="editing.draft.mtu" type="number" min="1280" max="1500" class="auth-input tabular-nums" placeholder="1420" />
          </label>
          <label class="md:col-span-3">
            <span class="auth-label">Notes</span>
            <input v-model="editing.draft.notes" class="auth-input" placeholder="Who runs the site's WireGuard, change window…" />
          </label>
        </div>
      </details>

      <ul v-if="showProblems && problems.length" class="list-disc space-y-1 pl-5 text-sm text-red-600 dark:text-red-400">
        <li v-for="p in problems" :key="p">{{ p }}</li>
      </ul>
      <div class="flex justify-end gap-2">
        <AppButton variant="ghost" @click="editing = null">Cancel</AppButton>
        <AppButton variant="primary" :disabled="saving" @click="save">{{ saving ? "Saving…" : isNew ? "Create tunnel" : "Save tunnel" }}</AppButton>
      </div>
    </section>

    <!-- Site setup -->
    <section v-if="setup" class="stat-card space-y-4">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 class="section-title">Site setup — {{ setup.tunnel.name }}</h2>
          <p class="text-sm text-muted-foreground">The complete wg0.conf for the site’s WireGuard server, and the steps to install it there by hand.</p>
        </div>
        <AppButton size="sm" variant="ghost" @click="setup = null">Close</AppButton>
      </div>

      <!-- Incomplete file: the site's private key is not stored here -->
      <!-- Keys being replaced: confirm first (the site disconnects until it installs the new file). -->
      <div v-if="rotateAsk" class="alert-warning space-y-2">
        <p>New site keys: the gateway switches right away, and a site already running this tunnel disconnects until it installs the new wg0.conf.</p>
        <label class="flex items-center gap-2">
          <input v-model="rotatePsk" type="checkbox" class="accent-[var(--primary)]" />
          Use a preshared key — a new one is generated (recommended; unticked, the tunnel stops using one)
        </label>
        <div class="flex gap-2">
          <AppButton size="sm" variant="primary" :disabled="rotating" @click="rotateKeys"><KeyRound class="h-3.5 w-3.5" :stroke-width="1.75" />{{ rotating ? "Generating…" : "Generate new keys" }}</AppButton>
          <AppButton size="sm" variant="ghost" @click="rotateAsk = false">Cancel</AppButton>
        </div>
      </div>
      <div v-else-if="setup.once.site_private_key || setup.once.preshared_key" class="alert-warning">
        The site's private key{{ setup.once.preshared_key ? " and the preshared key are" : " is" }} included below and shown only now — they are not stored here. Copy or download before closing; if
        lost, generate new site keys.
      </div>

      <div class="flex flex-wrap items-center justify-between gap-2">
        <h3 class="text-sm font-semibold text-foreground">wg0.conf for the site</h3>
        <div class="flex flex-wrap gap-2">
          <AppButton v-if="!setupComplete && !rotateAsk" size="sm" variant="ghost" title="Replace the site's keys and get a complete wg0.conf (the site must install it)" @click="rotateAsk = true">
            <KeyRound class="h-3.5 w-3.5" :stroke-width="1.75" />Generate new site keys
          </AppButton>
          <AppButton size="sm" @click="copyConf"><Copy class="h-3.5 w-3.5" :stroke-width="1.75" />Copy</AppButton>
          <AppButton size="sm" @click="downloadConf"><Download class="h-3.5 w-3.5" :stroke-width="1.75" />wg0.conf</AppButton>
        </div>
      </div>
      <p v-if="!setupComplete" class="text-xs text-muted-foreground">
        Lines in &lt;angle brackets&gt; are secrets this page never stores. A site that already runs this tunnel keeps its values; to set up a site, click
        <strong>Generate new site keys</strong> first — the file then has real keys to copy.
      </p>
      <pre class="max-h-[28rem] overflow-auto rounded-lg border border-border bg-muted p-3 font-mono text-xs leading-relaxed text-foreground">{{ setupText }}</pre>

      <div>
        <h3 class="mb-1 text-sm font-semibold text-foreground">Install it at the site (by hand)</h3>
        <p class="mb-2 text-xs text-muted-foreground">Nothing is installed for you. Whoever runs the site's WireGuard server copies the file there and runs these steps.</p>
        <ol class="space-y-2">
          <li v-for="(st, i) in manualSteps" :key="st.title" class="rounded-lg border border-border p-3">
            <div class="flex items-center gap-2">
              <span class="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-muted text-[11px] font-semibold tabular-nums text-foreground">{{ i + 1 }}</span>
              <span class="text-sm font-medium text-foreground">{{ st.title }}</span>
            </div>
            <div class="relative mt-2 pl-7">
              <pre class="overflow-x-auto rounded-md border border-border bg-muted p-2.5 pr-10 font-mono text-[11px] leading-relaxed text-foreground">{{ st.commands }}</pre>
              <button type="button" class="absolute right-1.5 top-1.5 rounded p-1 text-muted-foreground hover:bg-background hover:text-foreground" title="Copy" @click="copy(st.commands, 'Commands')">
                <Copy class="h-3.5 w-3.5" :stroke-width="1.75" />
              </button>
            </div>
          </li>
        </ol>
      </div>

      <details class="text-sm">
        <summary class="cursor-pointer font-medium text-foreground">Full checklist (firewall, database access)</summary>
        <ol class="mt-2 list-decimal space-y-1.5 pl-5 text-foreground">
          <li v-for="c in setup.config.checklist" :key="c">{{ c }}</li>
        </ol>
      </details>
    </section>

    <!-- List -->
    <section v-if="data?.configured" class="space-y-3">
      <div v-if="!list.length && !editing" class="rounded-xl border border-dashed border-border p-8 text-center">
        <Network class="mx-auto h-8 w-8 text-muted-foreground" :stroke-width="1.5" />
        <h2 class="mt-3 text-base font-semibold text-foreground">No tunnels yet</h2>
        <p class="mx-auto mt-1 max-w-md text-sm text-muted-foreground">Create one per site (source or target) whose database is not reachable from here. You get the exact configuration the site needs.</p>
        <AppButton variant="primary" class="mt-4" @click="startNew"><Plus class="h-4 w-4" :stroke-width="2" />New tunnel</AppButton>
      </div>

      <article v-for="t in list" :id="`tunnel-${t.id}`" :key="t.id" class="stat-card scroll-mt-4">
        <div class="flex flex-wrap items-start justify-between gap-3">
          <div class="min-w-0">
            <div class="flex items-center gap-2">
              <span class="h-2 w-2 rounded-full" :class="DOT[tunnelStatus(t.status, data?.gateway_reachable).tone]" aria-hidden="true" />
              <h3 class="truncate text-base font-semibold text-foreground">{{ t.name }}</h3>
            </div>
            <p class="mt-0.5 text-sm text-muted-foreground">{{ tunnelStatus(t.status, data?.gateway_reachable).text }}</p>
          </div>
          <div class="flex flex-wrap gap-1.5">
            <AppButton size="sm" @click="openSetup(t)">Site setup</AppButton>
            <AppButton size="sm" variant="ghost" @click="startEdit(t)"><Pencil class="h-3.5 w-3.5" :stroke-width="1.75" />Edit</AppButton>
            <AppButton size="sm" variant="ghost" title="Delete" @click="deleting = t"><Trash2 class="h-3.5 w-3.5" :stroke-width="1.75" /></AppButton>
          </div>
        </div>
        <dl class="mt-3 grid gap-x-6 gap-y-2 text-sm sm:grid-cols-2 lg:grid-cols-4">
          <div><dt class="text-xs text-muted-foreground">Site endpoint</dt><dd class="truncate font-mono text-xs text-foreground">{{ t.peer_endpoint }}</dd></div>
          <div><dt class="text-xs text-muted-foreground">Tunnel addresses</dt><dd class="font-mono text-xs text-foreground">{{ t.address }} ↔ {{ t.site_address || "—" }}</dd></div>
          <div><dt class="text-xs text-muted-foreground">Reaches</dt><dd class="truncate font-mono text-xs text-foreground" :title="t.allowed_ips.join(', ')">{{ t.allowed_ips.join(", ") }}</dd></div>
          <div><dt class="text-xs text-muted-foreground">Traffic</dt><dd class="text-xs tabular-nums text-foreground">↓ {{ fmtBytes(t.status?.rx_bytes) }} · ↑ {{ fmtBytes(t.status?.tx_bytes) }}</dd></div>
        </dl>
        <p v-if="t.status?.forwards?.length" class="mt-2 text-xs text-muted-foreground">
          In use for {{ t.status.forwards.map((f) => `${f.host}:${f.port}`).join(", ") }}
        </p>

        <TunnelReadiness v-model:open="readinessOpen[t.id]" class="mt-3" :tunnel="t" :gateway-reachable="!!data?.gateway_reachable" />
      </article>
    </section>

    <AppConfirmDialog
      :open="!!deleting"
      :title="`Delete ${deleting?.name}?`"
      confirm-label="Delete tunnel"
      danger
      @update:open="!$event && (deleting = null)"
      @confirm="confirmDelete"
    >
      <p>Connections that use it stop working until they choose another tunnel or Direct. The site keeps its WireGuard configuration.</p>
    </AppConfirmDialog>
  </div>
</template>
