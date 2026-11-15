<script setup lang="ts">
// MCP tools: everything a chat client can call, grouped, searchable, with read/write and arguments.
// Live from mcp-cdc (tools/list), so it always matches what clients see.
import { ChevronDown, ChevronRight, Copy, RefreshCw, Search } from "@lucide/vue"
import { MCP_EXAMPLE_PROMPTS, MCP_TOOL_GROUPS, groupOfTool } from "~/utils/mcpTools"
import type { McpTool } from "~/composables/useMcp"

definePageMeta({ layout: "default" })

const toast = useToast()
const { info, fetchInfo } = useMcp()
const loading = ref(true)
const load = async () => {
  loading.value = true
  await fetchInfo(true)
  loading.value = false
}
onMounted(load)

const search = ref("")
const access = ref<"all" | "read" | "write">("all")
const ACCESS = [
  { value: "all" as const, label: "All" },
  { value: "read" as const, label: "Read" },
  { value: "write" as const, label: "Write" },
]
const open = ref<Record<string, boolean>>({})

const tools = computed(() => info.value?.tools || [])
const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  return tools.value.filter(
    (t) =>
      (access.value === "all" || (access.value === "read") === t.read_only) &&
      (!q || t.name.toLowerCase().includes(q) || t.description.toLowerCase().includes(q)),
  )
})
const groups = computed(() => {
  const order = [...MCP_TOOL_GROUPS.map((g) => g.title), "Other"]
  const by: Record<string, McpTool[]> = {}
  for (const t of filtered.value) (by[groupOfTool(t.name)] ||= []).push(t)
  return order.filter((g) => by[g]?.length).map((g) => ({ title: g, tools: by[g]! }))
})

const args = (t: McpTool) => {
  const props = (t.input_schema?.properties || {}) as Record<string, any>
  const req: string[] = t.input_schema?.required || []
  return Object.entries(props).map(([name, p]) => ({
    name,
    required: req.includes(name),
    type: p?.enum ? p.enum.join(" | ") : p?.type || "any",
    description: p?.description || "",
  }))
}

const copy = async (text: string) => {
  try {
    await navigator.clipboard.writeText(text)
    toast.success("Copied")
  } catch {
    toast.error("Copy failed")
  }
}
</script>

<template>
  <div class="page-container">
    <AppPageHeader title="MCP tools" description="Everything a chat client can call through mcp-cdc.">
      <AppButton title="Refresh" @click="load"><RefreshCw class="h-4 w-4" :stroke-width="1.75" />Refresh</AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="loading && !info" kind="loading" message="Loading tools…" />
    <div v-else-if="info && !info.reachable" class="alert-warning">{{ info.error || "mcp-cdc is not reachable." }}</div>

    <template v-if="info?.reachable">
      <section class="stat-card">
        <div class="grid items-end gap-3 md:grid-cols-[minmax(0,1fr)_auto]">
          <label>
            <span class="auth-label">Search</span>
            <div class="relative">
              <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" :stroke-width="1.75" />
              <input v-model="search" class="auth-input !pl-9" placeholder="Name or description, e.g. latency, tunnel, redis" />
            </div>
          </label>
          <div>
            <AppFormLabel label="Access" hint="Read tools work with a read key. Write tools need a write key and ALLOW_WRITE_TOOLS=true." />
            <TransferSegmented v-model="access" label="Access" :options="ACCESS" />
          </div>
        </div>
        <p class="mt-3 text-xs text-muted-foreground">
          {{ filtered.length }} of {{ tools.length }} tools ·
          write tools {{ info.write_tools_enabled ? "enabled" : "disabled" }} on this server
        </p>
      </section>

      <section v-for="g in groups" :key="g.title" class="stat-card !p-0">
        <h2 class="section-title border-b border-border px-4 py-3">{{ g.title }} <span class="ml-1 text-xs font-normal text-muted-foreground">{{ g.tools.length }}</span></h2>
        <ul>
          <li v-for="t in g.tools" :key="t.name" class="border-b border-border last:border-b-0">
            <button type="button" class="flex w-full items-start gap-3 px-4 py-3 text-left hover:bg-muted/40" @click="open[t.name] = !open[t.name]">
              <component :is="open[t.name] ? ChevronDown : ChevronRight" class="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" :stroke-width="1.75" />
              <code class="w-64 shrink-0 font-mono text-xs text-foreground">{{ t.name }}</code>
              <span
                class="shrink-0 rounded px-1.5 py-0.5 text-[11px] font-medium"
                :class="t.read_only ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-400' : 'bg-amber-500/10 text-amber-700 dark:text-amber-400'"
              >{{ t.read_only ? "Read" : "Write" }}</span>
              <span class="min-w-0 flex-1 text-sm text-muted-foreground" :class="open[t.name] ? '' : 'line-clamp-1'">{{ t.description }}</span>
            </button>
            <div v-if="open[t.name]" class="px-4 pb-4 pl-11">
              <table v-if="args(t).length" class="w-full text-xs">
                <thead class="text-muted-foreground">
                  <tr><th class="py-1 text-left font-medium">Argument</th><th class="py-1 text-left font-medium">Type</th><th class="py-1 text-left font-medium">Description</th></tr>
                </thead>
                <tbody>
                  <tr v-for="a in args(t)" :key="a.name" class="border-t border-border">
                    <td class="py-1 pr-3 font-mono">{{ a.name }}<span v-if="a.required" class="text-red-600 dark:text-red-400">*</span></td>
                    <td class="py-1 pr-3 font-mono text-muted-foreground">{{ a.type }}</td>
                    <td class="py-1 text-muted-foreground">{{ a.description }}</td>
                  </tr>
                </tbody>
              </table>
              <p v-else class="text-xs text-muted-foreground">No arguments.</p>
            </div>
          </li>
        </ul>
      </section>
      <AppStateBlock v-if="!groups.length" kind="empty" message="No tools match." />

      <section class="grid gap-4 lg:grid-cols-2">
        <div class="stat-card space-y-2">
          <h2 class="section-title">Example prompts</h2>
          <ul class="space-y-1.5">
            <li v-for="p in MCP_EXAMPLE_PROMPTS" :key="p.ask" class="flex items-start justify-between gap-2 text-sm">
              <span>
                “{{ p.ask }}”
                <span class="block font-mono text-[11px] text-muted-foreground">{{ p.tools }}</span>
              </span>
              <AppButton size="sm" variant="ghost" title="Copy prompt" @click="copy(p.ask)"><Copy class="h-3.5 w-3.5" :stroke-width="1.75" /></AppButton>
            </li>
          </ul>
        </div>
        <div class="stat-card space-y-2">
          <h2 class="section-title">Prompts</h2>
          <ul class="space-y-1.5">
            <li v-for="p in info.prompts || []" :key="p.name" class="text-sm">
              <code class="font-mono text-xs text-foreground">{{ p.name }}</code>
              <span class="block text-muted-foreground">{{ p.description }}</span>
            </li>
          </ul>
        </div>
      </section>
    </template>
  </div>
</template>
