<script setup lang="ts">
// One host of a file sync (source or target): read row with the last test, or the edit form.
// SSH: the test logs in and reports OS, rsync flavour, path and host key. rsync daemon (rsync://,
// port 873): the test connects to the module and looks at the path; no shell, relay only.
import { AlertTriangle, CheckCircle2, Loader2, Lock, Pencil, PlugZap, Server, XCircle } from "@lucide/vue"
import { RSYNC_REDACTED, type RsyncEndpoint, type RsyncTestResult } from "~/composables/useRsync"
import { formatBytes } from "~/utils/bytes"

const props = defineProps<{
  role: "source" | "target"
  check: (e: RsyncEndpoint) => Promise<RsyncTestResult>
  startEditing?: boolean
}>()
const e = defineModel<RsyncEndpoint>({ required: true })
const result = defineModel<RsyncTestResult | null>("result", { default: null })
const editing = ref(!!props.startEditing || !e.value.host)
const testing = ref(false)

const saved = (v: string) => v === RSYNC_REDACTED
const daemon = computed(() => e.value.protocol === "daemon")
const protocolModel = computed({
  get: () => (e.value.protocol === "daemon" ? "daemon" : "ssh"),
  set: (v: "ssh" | "daemon") => {
    // Swap the usual port with the protocol; a custom port is kept.
    const port = (e.value.port || "").trim()
    if (v === "daemon" && (port === "" || port === "22")) e.value.port = "873"
    if (v === "ssh" && (port === "" || port === "873")) e.value.port = "22"
    e.value.protocol = v
    if (v === "daemon" && e.value.module === undefined) e.value.module = ""
  },
})
const protocolOptions = [
  { value: "ssh" as const, label: "SSH", help: "Log in with a key or password; rsync runs on the host." },
  {
    value: "daemon" as const,
    label: "rsync daemon (rsync://)",
    help: "The host runs rsync --daemon (port 873) with modules in rsyncd.conf. No SSH login; files go through the relay.",
  },
]
const canTest = computed(() => (daemon.value ? !!e.value.host && !!e.value.module?.trim() : !!e.value.host && !!e.value.username && !!e.value.path))
const pinned = computed(() => !!e.value.host_key_fingerprint?.trim())
const fingerprintOk = computed(() => !e.value.host_key_fingerprint || /^SHA256:[A-Za-z0-9+/]{43}=?$/.test(e.value.host_key_fingerprint.trim()))
const authModel = computed({ get: () => e.value.auth, set: (v: "key" | "password") => (e.value.auth = v) })
const authOptions = computed(() => [
  { value: "key" as const, label: "Private key", help: "OpenSSH or PEM key. An encrypted key needs its passphrase." },
  {
    value: "password" as const,
    label: "Password",
    help: props.role === "target" ? "A password target always goes through the relay: only a key can be lent to the source." : undefined,
  },
])
const pathHelp = computed(() =>
  props.role === "source"
    ? "End with / to copy what is inside the folder (/srv/www/). Without it the folder itself is copied into the target. Relative paths start in the login's home."
    : "A directory; created if missing. Relative paths start in the login's home.",
)

// Any change to the login invalidates the last test.
watch(
  () => [e.value.protocol, e.value.module, e.value.host, e.value.port, e.value.username, e.value.auth, e.value.private_key, e.value.password, e.value.passphrase, e.value.path, e.value.rsync_path],
  () => (result.value = null),
)

const test = async () => {
  testing.value = true
  try {
    result.value = await props.check(e.value)
  } catch (err: any) {
    result.value = { ok: false, target: e.value.host, error: err?.data?.error || err?.message || "Test failed" }
  } finally {
    testing.value = false
  }
}
const pinSeen = () => {
  if (result.value?.host_key_fingerprint) e.value.host_key_fingerprint = result.value.host_key_fingerprint
}
const facts = computed(() => {
  const r = result.value
  if (!r?.ok) return []
  return [
    daemon.value ? "rsync daemon" : [r.os === "Darwin" ? "macOS" : r.os, r.arch].filter(Boolean).join(" "),
    !daemon.value && r.rsync_version ? `${r.gnu_rsync ? "GNU rsync" : "rsync"} ${r.rsync_version.match(/version\s+(\S+)/)?.[1] || ""}`.trim() : "",
    r.path_kind === "missing" ? (props.role === "target" ? "path will be created" : "path missing") : r.path_kind === "dir" ? "folder exists" : r.path_kind || "",
    daemon.value ? `module ${e.value.module}` : "",
    r.free_bytes != null ? `${formatBytes(r.free_bytes)} free` : "",
    r.latency_ms != null ? `${r.latency_ms} ms` : "",
  ].filter(Boolean)
})
</script>

<template>
  <div class="rounded-lg border border-border">
    <div class="flex items-center gap-3 p-3">
      <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border bg-muted text-muted-foreground" aria-hidden="true">
        <Server class="h-4 w-4" :stroke-width="1.75" />
      </span>
      <div class="min-w-0 flex-1">
        <div class="truncate text-sm">
          <span class="font-medium text-foreground">{{ e.name || e.host || (role === "source" ? "Source host" : "Target host") }}</span>
          <span v-if="e.host && daemon" class="text-muted-foreground"> rsync://{{ e.username ? `${e.username}@` : "" }}{{ e.host }}:{{ e.port || "873" }}/{{ e.module || "?" }}</span>
          <span v-else-if="e.host" class="text-muted-foreground"> {{ e.username || "?" }}@{{ e.host }}:{{ e.port || "22" }}</span>
        </div>
        <div class="truncate font-mono text-xs text-muted-foreground">{{ daemon ? `${e.module || "?"}/${e.path || ""}` : e.path || "no path yet" }}</div>
      </div>
      <span v-if="e.host && daemon" class="hidden shrink-0 items-center gap-1 text-xs sm:inline-flex" :class="e.tunnel_id ? 'text-emerald-600 dark:text-emerald-400' : 'text-amber-600 dark:text-amber-400'">
        <component :is="e.tunnel_id ? Lock : AlertTriangle" class="h-3.5 w-3.5" :stroke-width="2" />
        {{ e.tunnel_id ? "Encrypted by the tunnel" : "Not encrypted" }}
      </span>
      <span
        v-else-if="e.host"
        class="hidden shrink-0 items-center gap-1 text-xs sm:inline-flex"
        :class="pinned ? 'text-emerald-600 dark:text-emerald-400' : 'text-amber-600 dark:text-amber-400'"
        :title="pinned ? e.host_key_fingerprint : 'Any host key is accepted. Test the login, then pin the key it shows.'"
      >
        <component :is="pinned ? Lock : AlertTriangle" class="h-3.5 w-3.5" :stroke-width="2" />
        {{ pinned ? "Host key pinned" : "Host key not pinned" }}
      </span>
      <AppButton size="sm" :disabled="testing || !canTest" @click="test">
        <Loader2 v-if="testing" class="h-3.5 w-3.5 animate-spin" :stroke-width="2" />
        <PlugZap v-else class="h-3.5 w-3.5" :stroke-width="1.75" />{{ daemon ? "Test connection" : "Test login" }}
      </AppButton>
      <AppButton size="sm" variant="ghost" :title="editing ? 'Close' : 'Edit host'" @click="editing = !editing">
        <Pencil class="h-3.5 w-3.5" :stroke-width="1.75" />{{ editing ? "Close" : "Edit" }}
      </AppButton>
    </div>

    <div v-if="result" class="border-t border-border px-3 py-2 text-sm" role="status">
      <div v-if="result.ok" class="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span class="inline-flex items-center gap-1 font-medium text-emerald-600 dark:text-emerald-400"><CheckCircle2 class="h-4 w-4" :stroke-width="2" />{{ daemon ? "Connected" : "Logged in" }}</span>
        <span v-for="f in facts" :key="f" class="text-xs tabular-nums text-muted-foreground">{{ f }}</span>
      </div>
      <div v-else class="space-y-0.5">
        <span class="inline-flex items-center gap-1 font-medium text-red-600 dark:text-red-400"><XCircle class="h-4 w-4" :stroke-width="2" />{{ result.error }}</span>
        <p v-if="result.hint" class="text-xs text-muted-foreground">{{ result.hint }}</p>
      </div>
      <p v-if="result.ok && result.warning" class="mt-1 text-xs text-amber-600 dark:text-amber-400">{{ result.warning }}</p>
      <div v-if="result.host_key_fingerprint && !result.host_key_verified && !pinned" class="mt-1 flex flex-wrap items-center gap-2 text-xs">
        <span class="text-muted-foreground">Host key:</span>
        <span class="break-all font-mono text-muted-foreground">{{ result.host_key_fingerprint }}</span>
        <AppButton size="sm" title="Use this fingerprint; save to keep it" @click="pinSeen"><Lock class="h-3.5 w-3.5" :stroke-width="1.75" />Pin this key</AppButton>
      </div>
    </div>

    <div v-if="editing" class="grid gap-3 border-t border-border p-3 sm:grid-cols-6">
      <div class="sm:col-span-6">
        <span class="auth-label !mb-1.5">Connect with</span>
        <TransferSegmented v-model="protocolModel" :options="protocolOptions" label="Protocol" />
      </div>
      <div class="sm:col-span-6">
        <TransferTunnelSelect v-model="e.tunnel_id" />
        <p v-if="e.tunnel_id" class="mt-1 text-xs text-muted-foreground">Through a tunnel, files always go via the relay (this server), never host to host.</p>
        <p v-else-if="daemon" class="mt-1 text-xs text-amber-600 dark:text-amber-400">
          The rsync daemon protocol is not encrypted (files and listings travel in plain text). Choose a WireGuard tunnel, or use it only on a trusted network.
        </p>
      </div>

      <template v-if="daemon">
        <label class="sm:col-span-2">
          <span class="auth-label !mb-1.5">Name</span>
          <input v-model="e.name" class="auth-input" :placeholder="role === 'source' ? 'web-01' : 'nas'" />
        </label>
        <label class="sm:col-span-2">
          <span class="auth-label !mb-1.5">Host</span>
          <input v-model.trim="e.host" class="auth-input" placeholder="10.66.1.1" spellcheck="false" />
        </label>
        <label class="sm:col-span-2">
          <span class="auth-label !mb-1.5">Port</span>
          <input v-model.trim="e.port" class="auth-input tabular-nums" inputmode="numeric" placeholder="873" />
        </label>
        <label class="sm:col-span-2">
          <span class="auth-label !mb-1.5">Module</span>
          <input v-model.trim="e.module" class="auth-input font-mono !text-xs" placeholder="data" spellcheck="false" />
          <span class="mt-1 block text-xs text-muted-foreground">A [name] from the site's rsyncd.conf.</span>
        </label>
        <label class="sm:col-span-4">
          <span class="auth-label !mb-1.5">{{ role === "source" ? "Copy from (inside the module)" : "Copy into (inside the module)" }}</span>
          <input v-model.trim="e.path" class="auth-input font-mono !text-xs" :placeholder="role === 'source' ? 'www/ (empty = whole module)' : 'backups/www (empty = module root)'" spellcheck="false" />
          <span class="mt-1 block text-xs text-muted-foreground">
            {{ role === "source" ? "End with / to copy what is inside the folder." : "The last folder is created if missing; its parents must exist." }}
          </span>
        </label>
        <label class="sm:col-span-3">
          <span class="auth-label !mb-1.5">Daemon user</span>
          <input v-model.trim="e.username" class="auth-input" placeholder="Empty for an anonymous module" spellcheck="false" />
          <span class="mt-1 block text-xs text-muted-foreground">From the module's <code class="font-mono">auth users</code>; not a system login.</span>
        </label>
        <div class="sm:col-span-3">
          <span class="auth-label !mb-1.5">Password</span>
          <div v-if="saved(e.password)" class="flex h-10 items-center justify-between rounded-xl border border-input px-4 text-sm">
            <span class="text-muted-foreground">Saved</span>
            <button type="button" class="btn-ghost text-xs" @click="e.password = ''">Replace</button>
          </div>
          <input v-else v-model="e.password" type="password" class="auth-input" :disabled="!e.username" :placeholder="e.username ? 'From the secrets file' : 'Set a user first'" autocomplete="new-password" aria-label="Daemon password" />
        </div>
      </template>

      <template v-else>
      <label class="sm:col-span-2">
        <span class="auth-label !mb-1.5">Name</span>
        <input v-model="e.name" class="auth-input" :placeholder="role === 'source' ? 'web-01' : 'mac-mini'" />
      </label>
      <label class="sm:col-span-3">
        <span class="auth-label !mb-1.5">Host</span>
        <input v-model.trim="e.host" class="auth-input" placeholder="server.example.com" spellcheck="false" />
      </label>
      <label class="sm:col-span-1">
        <span class="auth-label !mb-1.5">Port</span>
        <input v-model.trim="e.port" class="auth-input tabular-nums" inputmode="numeric" placeholder="22" />
      </label>
      <label class="sm:col-span-2">
        <span class="auth-label !mb-1.5">Username</span>
        <input v-model.trim="e.username" class="auth-input" placeholder="deploy" spellcheck="false" />
      </label>
      <label class="sm:col-span-4">
        <span class="auth-label !mb-1.5">{{ role === "source" ? "Copy from" : "Copy into" }}</span>
        <input v-model.trim="e.path" class="auth-input font-mono !text-xs" :placeholder="role === 'source' ? '/srv/www/' : 'backups/www'" spellcheck="false" />
        <span class="mt-1 block text-xs text-muted-foreground">{{ pathHelp }}</span>
      </label>

      <div class="sm:col-span-6">
        <span class="auth-label !mb-1.5">Sign in with</span>
        <TransferSegmented v-model="authModel" :options="authOptions" label="SSH authentication" />
      </div>
      <template v-if="e.auth === 'key'">
        <div class="sm:col-span-6">
          <span class="auth-label !mb-1.5">Private key</span>
          <div v-if="saved(e.private_key)" class="flex h-10 items-center justify-between rounded-xl border border-input px-4 text-sm">
            <span class="text-muted-foreground">Saved</span>
            <button type="button" class="btn-ghost text-xs" @click="e.private_key = ''">Replace</button>
          </div>
          <textarea v-else v-model="e.private_key" rows="4" class="auth-input font-mono !text-xs" placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" spellcheck="false" aria-label="Private key" />
        </div>
        <div class="sm:col-span-3">
          <span class="auth-label !mb-1.5">Key passphrase</span>
          <div v-if="saved(e.passphrase)" class="flex h-10 items-center justify-between rounded-xl border border-input px-4 text-sm">
            <span class="text-muted-foreground">Saved</span>
            <button type="button" class="btn-ghost text-xs" @click="e.passphrase = ''">Replace</button>
          </div>
          <input v-else v-model="e.passphrase" type="password" class="auth-input" placeholder="Only for an encrypted key" autocomplete="new-password" aria-label="Key passphrase" />
        </div>
      </template>
      <div v-else class="sm:col-span-3">
        <span class="auth-label !mb-1.5">Password</span>
        <div v-if="saved(e.password)" class="flex h-10 items-center justify-between rounded-xl border border-input px-4 text-sm">
          <span class="text-muted-foreground">Saved</span>
          <button type="button" class="btn-ghost text-xs" @click="e.password = ''">Replace</button>
        </div>
        <input v-else v-model="e.password" type="password" class="auth-input" autocomplete="new-password" aria-label="SSH password" />
      </div>

      <label class="sm:col-span-6">
        <span class="auth-label !mb-1.5">Host key fingerprint</span>
        <input
          v-model.trim="e.host_key_fingerprint"
          class="auth-input font-mono !text-xs"
          :class="fingerprintOk ? '' : '!border-red-500'"
          placeholder="SHA256:… (leave empty, test the login, then pin the key it shows)"
          spellcheck="false"
        />
        <span class="mt-1 block text-xs" :class="fingerprintOk ? 'text-muted-foreground' : 'text-red-600 dark:text-red-400'">
          {{ fingerprintOk ? "When set, a host with a different key is refused." : "Use the OpenSSH form: SHA256: followed by 43 characters." }}
        </span>
      </label>
      <label class="sm:col-span-3">
        <span class="auth-label !mb-1.5">rsync on this host</span>
        <input v-model.trim="e.rsync_path" class="auth-input font-mono !text-xs" placeholder="Found automatically" spellcheck="false" />
        <span class="mt-1 block text-xs text-muted-foreground">On a Mac, Homebrew rsync is preferred over the built-in one.</span>
      </label>
      <template v-if="role === 'target'">
        <label class="sm:col-span-2">
          <span class="auth-label !mb-1.5">Address from the source</span>
          <input v-model.trim="e.reach_host" class="auth-input" placeholder="Same as host" spellcheck="false" />
          <span class="mt-1 block text-xs text-muted-foreground">For direct copies, if the source uses another name or LAN address.</span>
        </label>
        <label class="sm:col-span-1">
          <span class="auth-label !mb-1.5">Port</span>
          <input v-model.trim="e.reach_port" class="auth-input tabular-nums" inputmode="numeric" :placeholder="e.port || '22'" />
        </label>
      </template>
      </template>
    </div>
  </div>
</template>
