<script setup lang="ts">
/**
 * One diagnosed error: which component failed (chip), what is wrong, how to fix it,
 * and the raw message folded underneath for whoever needs the stack trace.
 */
import {
  ChevronRight,
  Cpu,
  Database,
  FileJson,
  Plug,
  Server,
  SlidersHorizontal,
  Unplug,
  Wrench,
} from "@lucide/vue"
import type { ErrorDiagnosis } from "~/composables/useOperationsM3"

const props = withDefaults(
  defineProps<{
    diagnosis: ErrorDiagnosis
    /** Raw message / stack trace summary. */
    detail?: string | null
    /** Extra context shown before the title, e.g. the connector name. */
    context?: string | null
    compact?: boolean
  }>(),
  { detail: null, context: null, compact: false },
)

const ICONS = {
  source_database: Database,
  target_database: Database,
  network: Unplug,
  kafka_connect: Plug,
  kafka: Server,
  schema_registry: FileJson,
  pipeline_config: SlidersHorizontal,
  orchestrator: Cpu,
} as const

const icon = computed(() => ICONS[props.diagnosis.area] ?? Cpu)
</script>

<template>
  <div class="min-w-0 space-y-1.5" :class="compact ? 'text-xs' : 'text-sm'">
    <div class="flex flex-wrap items-center gap-2">
      <span
        class="inline-flex items-center gap-1 rounded-full border border-red-500/30 bg-red-500/10 px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wide text-red-700 dark:text-red-300"
      >
        <component :is="icon" class="h-3.5 w-3.5" :stroke-width="1.75" />
        {{ diagnosis.area_label }}
      </span>
      <span v-if="context" class="font-mono text-[11px] text-muted-foreground">{{ context }}</span>
    </div>
    <p class="font-semibold text-foreground">{{ diagnosis.title }}</p>
    <p class="flex items-start gap-1.5 text-foreground/90">
      <Wrench class="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" :stroke-width="1.75" />
      <span><span class="font-medium">Fix:</span> {{ diagnosis.fix }}</span>
    </p>
    <details v-if="detail" class="group text-xs text-muted-foreground">
      <summary class="flex cursor-pointer list-none items-center gap-1 hover:text-foreground">
        <ChevronRight class="h-3 w-3 transition-transform group-open:rotate-90" :stroke-width="2" />
        Technical detail
      </summary>
      <pre class="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/40 p-2 font-mono text-[11px]">{{ detail }}</pre>
    </details>
  </div>
</template>
