<script setup lang="ts">
// Source readiness panel for the pipeline wizard: recommended settings by
// database size plus the checks an operator runs on the source before Start.
// Content per engine lives in utils/sourceReadiness.ts.
import { ChevronDown, Copy, Database, TriangleAlert } from "@lucide/vue"
import { READINESS_TIERS, readinessGuide, splitTableList, type ReadinessTier } from "~/utils/sourceReadiness"

const props = withDefaults(
  defineProps<{
    sourceType: string
    captureMode: string
    /** Login the connector uses; checked for replication/CDC privileges. */
    username?: string
    /** Comma/newline separated `schema.table` list when specific tables are selected. */
    tables?: string
    database?: string
  }>(),
  { username: "", tables: "", database: "" },
)

const toast = useToast()
const open = ref(false)
const tier = ref<ReadinessTier>("small")

const guide = computed(() =>
  readinessGuide(props.sourceType, {
    username: props.username.trim(),
    tables: splitTableList(props.tables),
    database: props.database.trim(),
    captureMode: props.captureMode,
  }),
)

const tierLabel = (t: ReadinessTier) => `${t[0].toUpperCase()}${t.slice(1)}`
const tierOptions = computed(() =>
  READINESS_TIERS.map((t) => ({ value: t, label: tierLabel(t), help: guide.value?.tierHelp[t] })),
)
const script = computed(() => guide.value?.script(tier.value) ?? "")
const notes = computed(() => guide.value?.notes(tier.value) ?? [])

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
  <div v-if="guide" class="rounded-xl border border-border bg-muted/40">
    <button
      type="button"
      class="flex w-full items-center gap-3 p-4 text-left"
      :aria-expanded="open"
      @click="open = !open"
    >
      <Database class="h-5 w-5 shrink-0 text-brand-500" :stroke-width="1.75" />
      <div class="min-w-0 flex-1">
        <p class="text-sm font-medium text-foreground">{{ guide.title }}</p>
        <p class="text-xs text-muted-foreground">{{ guide.summary }}</p>
      </div>
      <ChevronDown class="h-4 w-4 shrink-0 transition-transform" :class="open ? 'rotate-180' : ''" />
    </button>

    <div v-if="guide.modeNote" class="flex items-start gap-2 border-t border-border px-4 py-3">
      <TriangleAlert class="mt-0.5 h-4 w-4 shrink-0 text-yellow-500" :stroke-width="1.75" />
      <p class="text-xs text-foreground">{{ guide.modeNote }}</p>
    </div>

    <div v-if="open" class="space-y-6 border-t border-border p-4">
      <section class="space-y-3">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <p class="text-sm font-medium">Recommended settings</p>
          <TransferSegmented v-model="tier" :options="tierOptions" label="Database size" />
        </div>

        <div class="overflow-x-auto rounded-lg border border-border bg-card">
          <table class="w-full text-xs">
            <thead class="text-left text-muted-foreground">
              <tr class="border-b border-border">
                <th class="px-3 py-2 font-medium">Parameter</th>
                <th v-for="t in READINESS_TIERS" :key="t" class="px-3 py-2 font-medium">{{ tierLabel(t) }}</th>
                <th class="px-3 py-2 font-medium">Applies</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="p in guide.params" :key="p.name" class="border-b border-border last:border-0">
                <td class="px-3 py-2">
                  <span class="flex items-center gap-1 font-mono">
                    {{ p.name }}
                    <AppInfoHint :text="p.info" />
                  </span>
                </td>
                <td
                  v-for="t in READINESS_TIERS"
                  :key="t"
                  class="px-3 py-2 font-mono"
                  :class="tier === t ? 'font-semibold text-foreground' : 'text-muted-foreground'"
                >
                  {{ p.values[t] }}
                </td>
                <td
                  class="px-3 py-2"
                  :class="p.disruptive ? 'text-yellow-600 dark:text-yellow-400' : 'text-muted-foreground'"
                >
                  {{ p.apply }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <div class="relative">
          <pre class="code-block whitespace-pre-wrap pr-12 text-xs">{{ script }}</pre>
          <AppButton
            size="icon"
            variant="ghost"
            class="absolute right-2 top-2"
            :title="`Copy ${guide.scriptLabel}`"
            @click="copy(script, guide.scriptLabel)"
          >
            <Copy class="h-3.5 w-3.5" />
          </AppButton>
        </div>
        <ul class="list-disc space-y-1 pl-5 text-xs text-muted-foreground">
          <li v-for="n in notes" :key="n">{{ n }}</li>
        </ul>
      </section>

      <section class="space-y-3">
        <p class="text-sm font-medium">Verify readiness</p>
        <div v-for="(c, i) in guide.checks" :key="c.key" class="space-y-2 rounded-lg border border-border bg-card p-3">
          <div class="flex items-center justify-between gap-2">
            <p class="text-sm font-medium">{{ i + 1 }} · {{ c.title }}</p>
            <AppButton size="sm" variant="ghost" @click="copy(c.sql, `${c.title} query`)">
              <Copy class="mr-1 h-3.5 w-3.5" /> Copy query
            </AppButton>
          </div>
          <pre class="code-block whitespace-pre-wrap text-xs">{{ c.sql }}</pre>
          <p class="text-xs font-medium text-foreground">Ready when</p>
          <ul class="list-disc space-y-0.5 pl-5 text-xs text-muted-foreground">
            <li v-for="r in c.ready" :key="r">{{ r }}</li>
          </ul>
          <template v-if="c.extra">
            <div class="flex items-center justify-between gap-2 pt-1">
              <p class="text-xs font-medium text-foreground">{{ c.extra.label }}</p>
              <AppButton size="sm" variant="ghost" @click="copy(c.extra.sql, c.extra.label)">
                <Copy class="mr-1 h-3.5 w-3.5" /> Copy
              </AppButton>
            </div>
            <pre class="code-block whitespace-pre-wrap text-xs">{{ c.extra.sql }}</pre>
          </template>
        </div>
      </section>
    </div>
  </div>
</template>
