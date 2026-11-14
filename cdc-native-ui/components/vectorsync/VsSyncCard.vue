<script setup lang="ts">
// One vector sync in the list: what feeds it, where vectors go, and whether it keeps up.
import { ArrowRight, Boxes, Database, Sparkles } from "@lucide/vue"
import type { VsProfile } from "~/composables/useVectorSync"
import { STATE_LABEL, ago, badgeStatus, fmtCount, tableOf } from "~/utils/vectorSync"

const props = defineProps<{ sync: VsProfile; pipelineName?: string }>()
defineEmits<{ open: [id: string] }>()

const st = computed(() => props.sync.status || { state: "unknown" })
const lag = computed(() => st.value.lag?.total ?? null)
const dead = computed(() => st.value.counters?.dead_letters_stored ?? 0)
const tables = computed(() => props.sync.topics.map(tableOf))
</script>

<template>
  <button
    type="button"
    class="stat-card group flex w-full flex-col gap-3 text-left transition-colors hover:border-primary/40 focus:outline-none focus-visible:ring-1 focus-visible:ring-ring"
    @click="$emit('open', sync.id)"
  >
    <div class="flex items-start justify-between gap-3">
      <div class="min-w-0">
        <p class="truncate text-sm font-semibold text-foreground">{{ sync.name }}</p>
        <p class="mt-0.5 truncate text-xs text-muted-foreground">
          {{ pipelineName || "Topics" }} · {{ tables.length }} table{{ tables.length === 1 ? "" : "s" }}
        </p>
      </div>
      <AppStatusBadge :status="badgeStatus(st.state)" :label="STATE_LABEL[st.state] || st.state" />
    </div>

    <!-- flow: tables → model → collection -->
    <div class="flex items-center gap-1.5 text-xs text-muted-foreground">
      <span class="tag-chip max-w-[34%] truncate" :title="tables.join(', ')"><Database class="h-3.5 w-3.5" :stroke-width="1.75" />{{ tables.slice(0, 2).join(", ") }}<template v-if="tables.length > 2"> +{{ tables.length - 2 }}</template></span>
      <ArrowRight class="h-3.5 w-3.5 shrink-0" :stroke-width="1.75" />
      <span class="tag-chip max-w-[30%] truncate" :title="sync.embedding.model"><Sparkles class="h-3.5 w-3.5" :stroke-width="1.75" />{{ sync.embedding.model }}</span>
      <ArrowRight class="h-3.5 w-3.5 shrink-0" :stroke-width="1.75" />
      <span class="tag-chip max-w-[30%] truncate font-mono" :title="sync.target.collection"><Boxes class="h-3.5 w-3.5" :stroke-width="1.75" />{{ sync.target.collection }}</span>
    </div>

    <div class="grid grid-cols-3 gap-2 border-t border-border pt-3">
      <div>
        <p class="text-[11px] text-muted-foreground">Documents</p>
        <p class="text-base font-semibold tabular-nums text-foreground">{{ fmtCount(st.documents) }}</p>
      </div>
      <div>
        <p class="text-[11px] text-muted-foreground">Behind</p>
        <p class="text-base font-semibold tabular-nums" :class="lag && lag > 1000 ? 'text-amber-700 dark:text-amber-400' : 'text-foreground'">
          {{ lag === null ? "—" : fmtCount(lag) }}
        </p>
      </div>
      <div>
        <p class="text-[11px] text-muted-foreground">Dead letters</p>
        <p class="text-base font-semibold tabular-nums" :class="dead ? 'text-red-700 dark:text-red-400' : 'text-foreground'">{{ fmtCount(dead) }}</p>
      </div>
    </div>
    <p class="truncate text-xs text-muted-foreground" :title="st.message">
      {{ st.message || "—" }}<template v-if="st.last_event_at"> · last change {{ ago(st.last_event_at) }}</template>
    </p>
  </button>
</template>
