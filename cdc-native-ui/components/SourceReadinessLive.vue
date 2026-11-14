<script setup lang="ts">
// "Check source": asks the source database whether it is ready for log-based CDC (ADR 0019).
// Read-only probes through jdbc-admin; settings the login may not read show as "Unknown" with
// the grant that would reveal them. Fixes are shown per check and as one script, never run.
import { CheckCircle2, CircleHelp, Copy, Info, Loader2, RefreshCw, ScanSearch, TriangleAlert, XCircle } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import { PLATFORM_LABEL, formatServer, readinessHeadline, sortChecks, type ReadinessReport, type ReadinessStatus } from "~/utils/cdcReadiness"

const props = defineProps<{
  check: () => Promise<ReadinessReport>
  /** Connection fields are incomplete. */
  disabled?: boolean
}>()

const toast = useToast()
const report = ref<ReadinessReport | null>(null)
const loading = ref(false)
const error = ref("")
const showParams = ref(false)
const showPassed = ref(false)

const run = async () => {
  loading.value = true
  error.value = ""
  try {
    report.value = await props.check()
  } catch (e: any) {
    report.value = null
    error.value = formatApiError(e, "Could not check the source")
  } finally {
    loading.value = false
  }
}

const headline = computed(() => (report.value ? readinessHeadline(report.value) : null))
const sorted = computed(() => (report.value ? sortChecks(report.value.checks) : []))
const attention = computed(() => sorted.value.filter((c) => c.status !== "pass"))
const passed = computed(() => sorted.value.filter((c) => c.status === "pass"))

const ICON: Record<ReadinessStatus, any> = { pass: CheckCircle2, warn: TriangleAlert, fail: XCircle, unknown: CircleHelp, info: Info }
const TONE: Record<ReadinessStatus, string> = {
  pass: "text-emerald-600 dark:text-emerald-400",
  warn: "text-amber-600 dark:text-amber-400",
  fail: "text-red-600 dark:text-red-400",
  unknown: "text-muted-foreground",
  info: "text-sky-600 dark:text-sky-400",
}
const LABEL: Record<ReadinessStatus, string> = { pass: "OK", warn: "Warning", fail: "Missing", unknown: "Unknown", info: "Info" }
const HEAD_TONE = { success: "alert-success", warning: "alert-warning", error: "alert-error", info: "alert-info" } as const

const copy = async (text: string, what: string) => {
  try {
    await navigator.clipboard.writeText(text)
    toast.success(`${what} copied`)
  } catch {
    toast.error("Copy failed")
  }
}
</script>

<template>
  <section class="space-y-3 border-t border-border p-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="min-w-0">
        <p class="text-sm font-medium text-foreground">Check this database</p>
        <p class="text-xs text-muted-foreground">Reads the live settings with the login above (read-only) and says what CDC still needs.</p>
      </div>
      <AppButton size="sm" :disabled="disabled || loading" @click="run">
        <component :is="loading ? Loader2 : report ? RefreshCw : ScanSearch" class="h-4 w-4" :class="loading && 'animate-spin'" :stroke-width="1.75" />
        {{ loading ? "Checking…" : report ? "Check again" : "Check source" }}
      </AppButton>
    </div>

    <p v-if="error" class="alert-error text-sm">{{ error }}</p>

    <template v-if="report">
      <div class="flex flex-wrap items-center gap-2 text-xs">
        <span v-if="formatServer(report.server)" class="tag-chip">{{ formatServer(report.server) }}</span>
        <span v-if="PLATFORM_LABEL[report.platform]" class="tag-chip">{{ PLATFORM_LABEL[report.platform] }}</span>
        <span v-if="report.applicable" class="text-muted-foreground">
          {{ report.summary.pass }} OK · {{ report.summary.fail }} missing · {{ report.summary.warn }} warnings · {{ report.summary.unknown }} unknown
        </span>
      </div>
      <p v-if="headline" class="text-sm" :class="HEAD_TONE[headline.tone]">{{ headline.text }}</p>
      <p v-if="report.note" class="text-xs text-muted-foreground">{{ report.note }}</p>

      <ul v-if="attention.length" class="divide-y divide-border rounded-lg border border-border bg-card">
        <li v-for="c in attention" :key="c.id" class="space-y-2 p-3">
          <div class="flex items-start gap-2">
            <component :is="ICON[c.status]" class="mt-0.5 h-4 w-4 shrink-0" :class="TONE[c.status]" :stroke-width="1.75" />
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-baseline gap-x-2">
                <p class="text-sm font-medium text-foreground">{{ c.title }}</p>
                <span class="text-[11px] font-medium" :class="TONE[c.status]">{{ LABEL[c.status] }}</span>
                <span v-if="c.value" class="font-mono text-[11px] text-muted-foreground">now: {{ c.value }}<template v-if="c.expected"> · needs: {{ c.expected }}</template></span>
              </div>
              <p class="mt-0.5 text-xs text-muted-foreground">{{ c.detail }}</p>
            </div>
          </div>
          <div v-if="c.fix" class="relative ml-6">
            <pre class="code-block whitespace-pre-wrap pr-10 text-xs">{{ c.fix }}</pre>
            <AppButton size="icon" variant="ghost" class="absolute right-1.5 top-1.5" title="Copy fix" @click="copy(c.fix, 'Fix')"><Copy class="h-3.5 w-3.5" /></AppButton>
          </div>
        </li>
      </ul>

      <div v-if="passed.length">
        <button type="button" class="text-xs font-medium text-muted-foreground hover:text-foreground" @click="showPassed = !showPassed">
          {{ showPassed ? "Hide" : "Show" }} {{ passed.length }} passed check{{ passed.length === 1 ? "" : "s" }}
        </button>
        <ul v-if="showPassed" class="mt-2 space-y-1">
          <li v-for="c in passed" :key="c.id" class="flex items-start gap-2 text-xs">
            <CheckCircle2 class="mt-0.5 h-3.5 w-3.5 shrink-0" :class="TONE.pass" :stroke-width="1.75" />
            <span><span class="font-medium text-foreground">{{ c.title }}</span><span v-if="c.value" class="font-mono text-muted-foreground"> · {{ c.value }}</span> — <span class="text-muted-foreground">{{ c.detail }}</span></span>
          </li>
        </ul>
      </div>

      <div v-if="report.fix_script" class="space-y-1">
        <div class="flex items-center justify-between">
          <p class="text-xs font-medium text-foreground">All fixes as one script <span class="font-normal text-muted-foreground">(review with your DBA; nothing was run)</span></p>
          <AppButton size="sm" variant="ghost" @click="copy(report.fix_script, 'Fix script')"><Copy class="mr-1 h-3.5 w-3.5" />Copy script</AppButton>
        </div>
      </div>

      <div v-if="report.parameters.length">
        <button type="button" class="text-xs font-medium text-muted-foreground hover:text-foreground" @click="showParams = !showParams">
          {{ showParams ? "Hide" : "Show" }} system parameters ({{ report.parameters.length }})
        </button>
        <div v-if="showParams" class="mt-2 overflow-x-auto rounded-lg border border-border bg-card">
          <table class="w-full text-xs">
            <tbody>
              <tr v-for="p in report.parameters" :key="p.name" class="border-b border-border last:border-0">
                <td class="px-3 py-1.5 font-mono text-muted-foreground">{{ p.name }}</td>
                <td class="px-3 py-1.5 font-mono text-foreground">{{ p.value || "—" }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </template>
  </section>
</template>
