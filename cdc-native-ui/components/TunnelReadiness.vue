<script setup lang="ts">
// Tunnel readiness (ADR 0010): the steps, ours and the site's, before tools can reach a database
// through a WireGuard tunnel — checked live (gateway, handshake, a TCP check through the tunnel),
// with the site's commands for each step. Rules: utils/tunnelReadiness.ts.
import { CheckCircle2, ChevronDown, Circle, CircleDashed, Copy, MinusCircle, PlugZap, XCircle } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import { ENGINES, readinessSummary, tunnelReadiness, type DbEngine, type ProbeLike } from "~/utils/tunnelReadiness"
import type { WireguardTunnel } from "~/composables/useWireguard"

const props = defineProps<{ tunnel: WireguardTunnel; gatewayReachable: boolean }>()
const open = defineModel<boolean>("open", { default: false })
const toast = useToast()
const api = useWireguard()

const engine = ref<DbEngine>("postgres")
const host = ref(props.tunnel.site_address?.split("/")[0] || "")
const port = ref<number | null>(5432)
const probing = ref(false)
const probe = ref<ProbeLike>(null)
const portTouched = ref(false)
watch(engine, (e) => {
  if (!portTouched.value) port.value = ENGINES.find((x) => x.value === e)!.port
})

const steps = computed(() => tunnelReadiness(props.tunnel, props.gatewayReachable, probe.value, engine.value))
const summary = computed(() => readinessSummary(steps.value))
const allReady = computed(() => summary.value.done === summary.value.total - 1 && steps.value.find((s) => s.key === "login")?.state === "manual")

const runCheck = async () => {
  if (!host.value.trim() || !port.value) return
  probing.value = true
  try {
    probe.value = await api.probe(props.tunnel.id, host.value.trim(), port.value)
    await api.fetchTunnels(true) // the probe may have caused the first handshake
  } catch (e: any) {
    probe.value = { ok: false, error: formatApiError(e, "Check failed"), target: `${host.value}:${port.value}` }
  } finally {
    probing.value = false
  }
}
const copy = async (text: string) => {
  await navigator.clipboard.writeText(text)
  toast.success("Commands copied")
}

const ICON = { ok: CheckCircle2, fail: XCircle, pending: CircleDashed, skip: MinusCircle, manual: Circle } as const
const ICON_CLASS = {
  ok: "text-emerald-600 dark:text-emerald-400",
  fail: "text-red-600 dark:text-red-400",
  pending: "text-muted-foreground",
  skip: "text-muted-foreground",
  manual: "text-amber-600 dark:text-amber-400",
} as const
const STATE_TEXT = { ok: "Done", fail: "Not ready", pending: "Waiting on an earlier step", skip: "Not needed", manual: "Check it" } as const
</script>

<template>
  <div class="rounded-lg border border-border">
    <button type="button" class="flex w-full items-center gap-3 px-3 py-2.5 text-left" :aria-expanded="open" @click="open = !open">
      <component :is="summary.blocking?.state === 'fail' ? XCircle : allReady ? CheckCircle2 : CircleDashed" class="h-4 w-4 shrink-0" :class="summary.blocking?.state === 'fail' ? ICON_CLASS.fail : allReady ? ICON_CLASS.ok : ICON_CLASS.manual" :stroke-width="2" />
      <span class="min-w-0 flex-1">
        <span class="text-sm font-medium text-foreground">Readiness · {{ summary.done }} of {{ summary.total }} ready</span>
        <span v-if="summary.blocking" class="block truncate text-xs text-muted-foreground">Next: {{ summary.blocking.title }}</span>
      </span>
      <ChevronDown class="h-4 w-4 shrink-0 text-muted-foreground transition-transform" :class="open ? 'rotate-180' : ''" />
    </button>

    <div v-if="open" class="space-y-4 border-t border-border p-3">
      <!-- live check through the tunnel -->
      <div class="space-y-2 rounded-lg bg-muted p-3">
        <div class="flex flex-wrap items-end gap-2">
          <div class="min-w-[10rem]">
            <span class="auth-label !mb-1">Database</span>
            <select v-model="engine" class="auth-input" aria-label="Database engine">
              <option v-for="e in ENGINES" :key="e.value" :value="e.value">{{ e.label }}</option>
            </select>
          </div>
          <label class="min-w-[12rem] flex-1">
            <span class="auth-label !mb-1">Host as seen from the site</span>
            <input v-model.trim="host" class="auth-input font-mono !text-xs" placeholder="192.168.10.5 or 10.66.1.1" />
          </label>
          <label class="w-24">
            <span class="auth-label !mb-1">Port</span>
            <input v-model.number="port" type="number" class="auth-input tabular-nums" @input="portTouched = true" />
          </label>
          <AppButton :disabled="probing || !host || !port" @click="runCheck">
            <PlugZap class="h-4 w-4" :stroke-width="1.75" />{{ probing ? "Checking…" : "Check through tunnel" }}
          </AppButton>
        </div>
        <p class="text-xs text-muted-foreground">Opens a TCP connection from the gateway, through the tunnel, to this address. It also starts the handshake if the site is ready.</p>
      </div>

      <!-- steps -->
      <ol class="space-y-2">
        <li v-for="(s, i) in steps" :key="s.key" class="rounded-lg border border-border p-3" :class="s.state === 'fail' ? 'row-error' : ''">
          <div class="flex items-start gap-2.5">
            <component :is="ICON[s.state]" class="mt-0.5 h-4 w-4 shrink-0" :class="ICON_CLASS[s.state]" :stroke-width="2" />
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-x-2 gap-y-0.5">
                <span class="text-sm font-medium text-foreground">{{ i + 1 }}. {{ s.title }}</span>
                <span class="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">{{ s.who }}</span>
                <span class="text-xs" :class="ICON_CLASS[s.state]">{{ STATE_TEXT[s.state] }}</span>
              </div>
              <p class="mt-0.5 text-xs text-muted-foreground">{{ s.detail }}</p>
              <p v-if="s.fix" class="mt-1 text-xs text-foreground"><span class="font-medium">Fix:</span> {{ s.fix }}</p>
              <details v-if="s.commands" class="group mt-2" :open="s.state === 'fail'">
                <summary class="cursor-pointer list-none text-xs font-medium text-primary hover:underline">Commands for the site</summary>
                <div class="relative mt-1.5">
                  <pre class="overflow-x-auto rounded-md border border-border bg-background p-2.5 pr-10 font-mono text-[11px] leading-relaxed text-foreground">{{ s.commands }}</pre>
                  <button type="button" class="absolute right-1.5 top-1.5 rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground" title="Copy" @click="copy(s.commands!)">
                    <Copy class="h-3.5 w-3.5" :stroke-width="1.75" />
                  </button>
                </div>
              </details>
            </div>
          </div>
        </li>
      </ol>
    </div>
  </div>
</template>
