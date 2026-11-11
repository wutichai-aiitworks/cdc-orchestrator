<script setup lang="ts">
// Before creating a tunnel (ADR 0010): what the site collects and prepares — one open UDP port above
// all — with commands for its platform, a UDP echo test run by the gateway, and a Markdown handoff.
// Rules: utils/sitePreparation.ts.
import { CheckCircle2, ChevronDown, ClipboardList, Copy, Download, Radio, XCircle } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import {
  DEFAULT_PREP,
  EDGES,
  FIREWALLS,
  HOST_OS,
  INFO_TO_COLLECT,
  preparationSteps,
  siteChecklistMarkdown,
  udpTestSummary,
  type PrepOptions,
  type UdpTestResult,
} from "~/utils/sitePreparation"

const props = defineProps<{ siteName?: string }>()
/** The site's endpoint (host:port); shared with the tunnel form when used inside it. */
const endpoint = defineModel<string>("endpoint", { default: "" })
const open = defineModel<boolean>("open", { default: true })
const toast = useToast()

const opts = reactive<PrepOptions>({ ...DEFAULT_PREP })
const port = computed(() => Number(endpoint.value.split(":").pop()) || opts.port)
const steps = computed(() => preparationSteps({ ...opts, port: port.value }))
const markdown = computed(() => siteChecklistMarkdown({ ...opts, port: port.value }, props.siteName?.trim() || "<site>"))

const testing = ref(false)
const result = ref<UdpTestResult | null>(null)
const runTest = async () => {
  if (!/^[^\s:]+:\d{1,5}$/.test(endpoint.value.trim())) {
    toast.error("Enter the site endpoint as host:port first")
    return
  }
  testing.value = true
  result.value = null
  try {
    result.value = await $fetch<UdpTestResult>("/api/cdc/wireguard/udp-test", { method: "POST", body: { endpoint: endpoint.value.trim() } })
  } catch (e: any) {
    result.value = { ok: false, error: formatApiError(e, "UDP test failed"), endpoint: endpoint.value }
  } finally {
    testing.value = false
  }
}
watch(endpoint, () => (result.value = null))

const copy = async (text: string, what: string) => {
  await navigator.clipboard.writeText(text)
  toast.success(`${what} copied`)
}
const download = () => {
  const a = document.createElement("a")
  a.href = URL.createObjectURL(new Blob([markdown.value], { type: "text/markdown" }))
  a.download = `wireguard-site-preparation${props.siteName ? `-${props.siteName.trim().replace(/[^\w.-]+/g, "_")}` : ""}.md`
  a.click()
  URL.revokeObjectURL(a.href)
}
</script>

<template>
  <div class="rounded-lg border border-border">
    <button type="button" class="flex w-full items-center gap-3 px-3 py-2.5 text-left" :aria-expanded="open" @click="open = !open">
      <ClipboardList class="h-4 w-4 shrink-0 text-primary" :stroke-width="1.75" />
      <span class="min-w-0 flex-1">
        <span class="text-sm font-medium text-foreground">Before you create the tunnel: prepare the site</span>
        <span class="block text-xs text-muted-foreground">Information to collect, the UDP port to open (UDP {{ port }}), and a test that the port is reachable.</span>
      </span>
      <ChevronDown class="h-4 w-4 shrink-0 text-muted-foreground transition-transform" :class="open ? 'rotate-180' : ''" />
    </button>

    <div v-if="open" class="space-y-5 border-t border-border p-3">
      <!-- platform -->
      <div class="grid gap-3 sm:grid-cols-3">
        <label><span class="auth-label !mb-1">Site's OS</span>
          <select v-model="opts.os" class="auth-input"><option v-for="o in HOST_OS" :key="o.value" :value="o.value">{{ o.label }}</option></select>
        </label>
        <label><span class="auth-label !mb-1">Host firewall</span>
          <select v-model="opts.firewall" class="auth-input"><option v-for="o in FIREWALLS" :key="o.value" :value="o.value">{{ o.label }}</option></select>
        </label>
        <label><span class="auth-label !mb-1">In front of the host</span>
          <select v-model="opts.edge" class="auth-input"><option v-for="o in EDGES" :key="o.value" :value="o.value">{{ o.label }}</option></select>
        </label>
      </div>

      <!-- information -->
      <section>
        <h3 class="mb-2 text-sm font-semibold text-foreground">Information to collect from the site</h3>
        <div class="overflow-x-auto rounded-lg border border-border">
          <table class="w-full text-xs">
            <tbody>
              <tr v-for="i in INFO_TO_COLLECT" :key="i.label" class="border-b border-border last:border-0 align-top">
                <td class="w-1/4 px-3 py-2 font-medium text-foreground">{{ i.label }}</td>
                <td class="px-3 py-2 text-muted-foreground">{{ i.why }}</td>
                <td class="whitespace-nowrap px-3 py-2 font-mono text-[11px] text-foreground">{{ i.example }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <!-- steps -->
      <section>
        <h3 class="mb-2 text-sm font-semibold text-foreground">Steps at the site</h3>
        <ol class="space-y-2">
          <li v-for="(s, i) in steps" :key="s.key" class="rounded-lg border border-border p-3">
            <div class="flex flex-wrap items-center gap-2">
              <span class="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-muted text-[11px] font-semibold tabular-nums text-foreground">{{ i + 1 }}</span>
              <span class="text-sm font-medium text-foreground">{{ s.title }}</span>
              <span v-if="s.optional" class="rounded bg-muted px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">Optional</span>
            </div>
            <p class="mt-1 pl-7 text-xs text-muted-foreground">{{ s.detail }}</p>
            <div v-if="s.commands" class="relative mt-2 pl-7">
              <pre class="overflow-x-auto rounded-md border border-border bg-muted p-2.5 pr-10 font-mono text-[11px] leading-relaxed text-foreground">{{ s.commands }}</pre>
              <button type="button" class="absolute right-1.5 top-1.5 rounded p-1 text-muted-foreground hover:bg-background hover:text-foreground" title="Copy" @click="copy(s.commands!, 'Commands')">
                <Copy class="h-3.5 w-3.5" :stroke-width="1.75" />
              </button>
            </div>

            <!-- the UDP test lives on its step -->
            <div v-if="s.key === 'udptest'" class="mt-3 space-y-2 pl-7">
              <div class="flex flex-wrap items-end gap-2">
                <label class="min-w-[16rem] flex-1">
                  <span class="auth-label !mb-1">Site endpoint</span>
                  <input v-model.trim="endpoint" class="auth-input font-mono !text-xs" placeholder="vpn.site.example:51820" spellcheck="false" />
                </label>
                <AppButton :disabled="testing || !endpoint" @click="runTest">
                  <Radio class="h-4 w-4" :class="testing ? 'animate-pulse' : ''" :stroke-width="1.75" />{{ testing ? "Sending…" : "Test UDP port" }}
                </AppButton>
              </div>
              <div v-if="result" class="flex items-start gap-2 rounded-md p-2 text-xs" :class="udpTestSummary(result).tone === 'ok' ? 'alert-success' : udpTestSummary(result).tone === 'err' ? 'alert-error' : 'alert-warning'">
                <component :is="result.ok ? CheckCircle2 : XCircle" class="mt-0.5 h-4 w-4 shrink-0" :stroke-width="2" />
                <div>
                  <p class="font-medium">{{ udpTestSummary(result).text }}</p>
                  <p v-if="result.hint" class="mt-0.5">{{ result.hint }}</p>
                  <p v-if="result.sent" class="mt-0.5 font-mono">Text sent: {{ result.sent }}</p>
                </div>
              </div>
            </div>
          </li>
        </ol>
      </section>

      <div class="flex flex-wrap items-center justify-between gap-2 border-t border-border pt-3">
        <p class="text-xs text-muted-foreground">Send this to whoever runs the site; it includes the commands for the platform chosen above.</p>
        <div class="flex gap-2">
          <AppButton size="sm" @click="copy(markdown, 'Checklist')"><Copy class="h-3.5 w-3.5" :stroke-width="1.75" />Copy checklist</AppButton>
          <AppButton size="sm" @click="download"><Download class="h-3.5 w-3.5" :stroke-width="1.75" />Download .md</AppButton>
        </div>
      </div>
    </div>
  </div>
</template>
