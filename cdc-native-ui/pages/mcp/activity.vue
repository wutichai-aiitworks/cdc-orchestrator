<script setup lang="ts">
// MCP activity: what chat clients did through mcp-cdc, from the orchestrator audit trail
// (actor = "mcp:<client name>"). Reads only.
import { RefreshCw } from "@lucide/vue"

definePageMeta({ layout: "default" })

type AuditRow = {
  audit_id: string
  timestamp: string
  actor: string
  action: string
  outcome: string
  detail?: string
  profile_id?: string | null
  profile_name?: string | null
}

const rows = ref<AuditRow[]>([])
const loading = ref(true)
const error = ref("")
const outcome = ref<"all" | "success" | "failure">("all")
const OUTCOMES = [
  { value: "all" as const, label: "All" },
  { value: "success" as const, label: "Success" },
  { value: "failure" as const, label: "Failed" },
]

const load = async () => {
  loading.value = true
  try {
    const query: Record<string, string | number> = { actor: "mcp:", limit: 200 }
    if (outcome.value !== "all") query.outcome = outcome.value
    const res = await $fetch<{ items: AuditRow[] }>("/api/cdc/audit/events", { query })
    rows.value = res.items || []
    error.value = ""
  } catch (e: any) {
    error.value = e?.data?.message || "Could not load the audit trail."
  } finally {
    loading.value = false
  }
}
onMounted(load)
watch(outcome, load)

const client = (actor: string) => actor.replace(/^mcp:/, "") || "mcp"
const when = (ts: string) => new Date(ts).toLocaleString()
const clients = computed(() => [...new Set(rows.value.map((r) => client(r.actor)))])
</script>

<template>
  <div class="page-container">
    <AppPageHeader title="MCP activity" description="Actions chat clients performed through mcp-cdc.">
      <AppButton title="Refresh" @click="load"><RefreshCw class="h-4 w-4" :stroke-width="1.75" />Refresh</AppButton>
    </AppPageHeader>

    <section class="stat-card">
      <div class="flex flex-wrap items-end justify-between gap-3">
        <div>
          <AppFormLabel label="Outcome" hint="Reads (status questions) are not audited; this lists changes and actions such as start, stop, create and test." />
          <TransferSegmented v-model="outcome" label="Outcome" :options="OUTCOMES" />
        </div>
        <p class="text-xs text-muted-foreground">{{ rows.length }} events · {{ clients.length }} client{{ clients.length === 1 ? "" : "s" }}</p>
      </div>
    </section>

    <AppStateBlock v-if="loading && !rows.length" kind="loading" message="Loading activity…" />
    <AppStateBlock v-else-if="error" kind="error" :message="error" />
    <AppStateBlock v-else-if="!rows.length" kind="empty" message="No MCP actions recorded yet." />
    <section v-else class="stat-card !p-0 overflow-x-auto">
      <table class="w-full text-sm">
        <thead class="text-xs text-muted-foreground">
          <tr class="border-b border-border">
            <th class="px-4 py-2 text-left font-medium">Time</th>
            <th class="px-4 py-2 text-left font-medium">Client</th>
            <th class="px-4 py-2 text-left font-medium">Action</th>
            <th class="px-4 py-2 text-left font-medium">Pipeline</th>
            <th class="px-4 py-2 text-left font-medium">Outcome</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="r in rows" :key="r.audit_id" class="border-b border-border last:border-b-0">
            <td class="whitespace-nowrap px-4 py-2 text-xs text-muted-foreground">{{ when(r.timestamp) }}</td>
            <td class="px-4 py-2 font-mono text-xs">{{ client(r.actor) }}</td>
            <td class="px-4 py-2 font-mono text-xs">{{ r.action }}</td>
            <td class="px-4 py-2 text-xs">{{ r.profile_name || "—" }}</td>
            <td class="px-4 py-2">
              <span
                class="rounded px-1.5 py-0.5 text-[11px] font-medium"
                :class="r.outcome === 'success' ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-400' : 'bg-red-500/10 text-red-700 dark:text-red-400'"
              >{{ r.outcome }}</span>
            </td>
          </tr>
        </tbody>
      </table>
    </section>
  </div>
</template>
