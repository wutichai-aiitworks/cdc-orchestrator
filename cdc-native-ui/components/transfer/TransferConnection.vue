<script setup lang="ts">
// One connection of a transfer: read view (with the last check result) or edit view.
// Engines differ only in fields: a file source has a path; Redis has no database; usernames can be
// optional. Saved passwords come back as ******** and show as "Saved" with Replace; changing the
// host, port, user or engine asks for the password again (the API enforces the same rule).
import { AlertTriangle, CheckCircle2, Eye, EyeOff, Loader2, Pencil, PlugZap, XCircle } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import { loginKeyOf, relativeTime } from "~/utils/transferView"
import type { ConnCheck } from "~/utils/transferTools/types"

export type ConnValue = {
  type: string
  host: string
  port: string
  database: string
  username: string
  password: string
  schema?: string
  /** WireGuard tunnel to the site (ADR 0010); host/port are as seen from the site. */
  tunnel_id?: string | null
}

const conn = defineModel<ConnValue>({ required: true })
const result = defineModel<ConnCheck | null>("result", { default: null })
const props = withDefaults(
  defineProps<{
    engine: string
    defaultPort?: string
    hostPlaceholder?: string
    /** Database field; null hides it (Redis). */
    database?: { label: string; placeholder?: string } | null
    /** Path field instead of host/port/user/password (SQLite, CSV). Stored in `database`. */
    file?: { label: string; placeholder?: string; help?: string } | null
    usernameOptional?: boolean
    usernamePlaceholder?: string
    check: (c: ConnValue) => Promise<Omit<ConnCheck, "at">>
    checkLabel?: string
    checkNote?: string
    /** Read-view extras such as "TLS required" or "cluster". */
    summaryExtra?: string
    /** Changes to settings outside `conn` (TLS, auth database…) that invalidate the last check. */
    watchKey?: string
    savedLogin?: string
    startEditing?: boolean
  }>(),
  {
    defaultPort: "",
    hostPlaceholder: "db.example.internal",
    database: () => ({ label: "Database" }),
    file: null,
    usernamePlaceholder: "",
    checkLabel: "Test connection",
    checkNote: "",
    summaryExtra: "",
    watchKey: "",
    savedLogin: undefined,
  },
)

const REDACTED = "********"
const editing = ref(!!props.startEditing)
const checking = ref(false)
const showPassword = ref(false)
const pwInput = ref<HTMLInputElement | null>(null)

const loginKey = computed(() => loginKeyOf(conn.value, props.defaultPort))
const passwordSaved = computed(() => conn.value.password === REDACTED)
const complete = computed(() =>
  props.file
    ? !!conn.value.database.trim()
    : !!conn.value.host.trim() && (!props.database || !!conn.value.database.trim()) && (props.usernameOptional || !!conn.value.username.trim()),
)

watch(loginKey, (key) => {
  if (props.savedLogin === undefined) return
  if (key !== props.savedLogin && passwordSaved.value) conn.value.password = ""
  else if (key === props.savedLogin && conn.value.password === "") conn.value.password = REDACTED
})
const needsPasswordAgain = computed(() => props.savedLogin !== undefined && loginKey.value !== props.savedLogin && !conn.value.password && !props.file)

// A check describes the settings it ran with; editing invalidates it. (A string key: an array
// getter is a new value on every reload and would always fire.) Saving swaps a typed password
// for ******** — the same password, so that does not invalidate.
watch(() => `${loginKey.value}|${conn.value.database}|${props.watchKey}|${conn.value.tunnel_id || ""}`, () => (result.value = null))
const { tunnelName } = useWireguard()
watch(
  () => conn.value.password,
  (pw) => {
    if (pw !== REDACTED) result.value = null
  },
)

const replacePassword = () => {
  conn.value.password = ""
  nextTick(() => pwInput.value?.focus())
}

const run = async () => {
  checking.value = true
  try {
    const res = await props.check({ ...conn.value })
    result.value = { ...res, at: new Date().toISOString() }
    if (res.ok && complete.value) editing.value = false
  } catch (e: any) {
    result.value = { ok: false, error: formatApiError(e, "Check failed"), at: new Date().toISOString() }
  } finally {
    checking.value = false
  }
}

const portText = computed(() => conn.value.port || props.defaultPort)
</script>

<template>
  <div class="space-y-3">
    <!-- read view -->
    <div v-if="!editing && complete" class="flex flex-wrap items-start gap-3 rounded-lg border border-border p-3">
      <TransferEngineMark :engine="engine" size="md" />
      <div class="min-w-[14rem] flex-1">
        <template v-if="file">
          <div class="truncate font-mono text-sm font-semibold text-foreground" :title="conn.database">{{ conn.database }}</div>
          <div class="text-xs text-muted-foreground">{{ file.label }} inside the tool's container</div>
        </template>
        <template v-else>
          <div class="truncate text-sm font-semibold text-foreground">{{ conn.host }}<template v-if="portText">:{{ portText }}</template></div>
          <div class="truncate text-xs text-muted-foreground">
            <template v-if="database && conn.database">{{ database.label }} <span class="text-foreground">{{ conn.database }}</span></template>
            <template v-if="conn.username"><template v-if="database && conn.database"> as </template><template v-else>User </template><span class="text-foreground">{{ conn.username }}</span></template>
            <template v-if="summaryExtra">{{ (database && conn.database) || conn.username ? ", " : "" }}{{ summaryExtra }}</template>
          </div>
        </template>
        <div v-if="conn.tunnel_id" class="mt-1 inline-flex items-center gap-1 rounded-md bg-brand-500/10 px-1.5 py-0.5 text-[11px] font-medium text-primary">
          via WireGuard: {{ tunnelName(conn.tunnel_id) || "tunnel" }}
        </div>
        <div v-if="needsPasswordAgain" class="mt-1 flex items-center gap-1 text-xs text-amber-600 dark:text-amber-400">
          <AlertTriangle class="h-3.5 w-3.5" :stroke-width="2" /> Login changed, enter the password again
        </div>
      </div>
      <div class="flex shrink-0 gap-1.5">
        <AppButton size="sm" :disabled="checking" @click="run">
          <Loader2 v-if="checking" class="h-3.5 w-3.5 animate-spin" :stroke-width="2" />
          <PlugZap v-else class="h-3.5 w-3.5" :stroke-width="1.75" />{{ checkLabel }}
        </AppButton>
        <AppButton size="sm" variant="ghost" @click="editing = true"><Pencil class="h-3.5 w-3.5" :stroke-width="1.75" />Edit</AppButton>
      </div>
    </div>

    <!-- edit view -->
    <div v-else class="grid gap-3 sm:grid-cols-6">
      <label v-if="file" class="sm:col-span-6">
        <span class="auth-label !mb-1.5">{{ file.label }}</span>
        <input v-model.trim="conn.database" class="auth-input font-mono !text-xs" :placeholder="file.placeholder" spellcheck="false" />
        <span v-if="file.help" class="mt-1 block text-xs text-muted-foreground">{{ file.help }}</span>
      </label>
      <template v-else>
        <div class="sm:col-span-6"><TransferTunnelSelect v-model="conn.tunnel_id" /></div>
        <label class="sm:col-span-4">
          <span class="auth-label !mb-1.5">Host</span>
          <input v-model.trim="conn.host" class="auth-input" :placeholder="hostPlaceholder" autocomplete="off" spellcheck="false" />
        </label>
        <label class="sm:col-span-2">
          <span class="auth-label !mb-1.5">Port</span>
          <input v-model.trim="conn.port" class="auth-input tabular-nums" inputmode="numeric" :placeholder="defaultPort" />
        </label>
        <label v-if="database" class="sm:col-span-3">
          <span class="auth-label !mb-1.5">{{ database.label }}</span>
          <input v-model.trim="conn.database" class="auth-input" :placeholder="database.placeholder" autocomplete="off" spellcheck="false" />
        </label>
        <label :class="database ? 'sm:col-span-3' : 'sm:col-span-3'">
          <span class="auth-label !mb-1.5">Username<span v-if="usernameOptional" class="font-normal"> (optional)</span></span>
          <input v-model.trim="conn.username" class="auth-input" :placeholder="usernamePlaceholder" autocomplete="off" spellcheck="false" />
        </label>
        <div class="sm:col-span-3">
          <span class="auth-label !mb-1.5">Password</span>
          <div v-if="passwordSaved" class="flex h-10 items-center justify-between rounded-xl border border-input px-4 text-sm">
            <span class="text-muted-foreground">Saved</span>
            <button type="button" class="btn-ghost text-xs" @click="replacePassword">Replace</button>
          </div>
          <div v-else class="relative">
            <input
              ref="pwInput"
              v-model="conn.password"
              :type="showPassword ? 'text' : 'password'"
              class="auth-input !pr-10"
              autocomplete="new-password"
              :placeholder="needsPasswordAgain ? 'Login changed, enter it again' : usernameOptional ? 'Optional' : 'Password'"
              aria-label="Password"
            />
            <button
              type="button"
              class="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              :aria-label="showPassword ? 'Hide password' : 'Show password'"
              @click="showPassword = !showPassword"
            >
              <component :is="showPassword ? EyeOff : Eye" class="h-4 w-4" :stroke-width="1.75" />
            </button>
          </div>
        </div>
      </template>
      <div v-if="$slots.extra" class="sm:col-span-6"><slot name="extra" /></div>
      <div class="flex flex-wrap items-center gap-2 sm:col-span-6">
        <AppButton size="sm" :disabled="checking || !complete" :title="complete ? '' : 'Fill in the connection first'" @click="run">
          <Loader2 v-if="checking" class="h-3.5 w-3.5 animate-spin" :stroke-width="2" />
          <PlugZap v-else class="h-3.5 w-3.5" :stroke-width="1.75" />{{ checkLabel }}
        </AppButton>
        <AppButton v-if="complete" size="sm" variant="ghost" @click="editing = false">Done</AppButton>
        <span v-if="checkNote" class="text-xs text-muted-foreground">{{ checkNote }}</span>
      </div>
    </div>

    <!-- last check -->
    <div v-if="result" role="status" aria-live="polite" class="text-xs">
      <div v-if="result.ok" class="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span class="inline-flex items-center gap-1 font-medium text-emerald-600 dark:text-emerald-400">
          <CheckCircle2 class="h-3.5 w-3.5" :stroke-width="2" /> {{ result.facts?.length ? `Connected ${relativeTime(result.at)}` : `Looks right, checked ${relativeTime(result.at)}` }}
        </span>
        <span v-for="f in result.facts || []" :key="f" class="tabular-nums text-muted-foreground">{{ f }}</span>
      </div>
      <div v-if="result.ok && result.warning" class="mt-1.5 flex items-start gap-1.5 text-amber-600 dark:text-amber-400">
        <AlertTriangle class="mt-0.5 h-3.5 w-3.5 shrink-0" :stroke-width="2" /><span>{{ result.warning }}</span>
      </div>
      <div v-if="!result.ok" class="rounded-lg border border-red-500/30 bg-red-500/5 p-3">
        <div class="flex items-start gap-1.5 font-medium text-red-600 dark:text-red-400">
          <XCircle class="mt-0.5 h-3.5 w-3.5 shrink-0" :stroke-width="2" />
          <span>{{ result.hint || result.error || "Could not connect" }}</span>
        </div>
        <div v-if="result.hint && result.error" class="mt-1 whitespace-pre-wrap break-all pl-5 font-mono text-[11px] text-muted-foreground">{{ result.error }}</div>
      </div>
    </div>
  </div>
</template>
