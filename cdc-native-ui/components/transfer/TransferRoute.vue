<script setup lang="ts">
// Source → steps → destination(s). The one distinctive element of a transfer page: the line is
// static, and animates in the primary colour only while a run is live (off under reduced motion).
import { AlertTriangle, Check, X } from "@lucide/vue"
import type { RouteEnd } from "~/utils/transferTools/types"
import type { RouteStep } from "~/utils/transferView"

defineProps<{
  source: RouteEnd
  steps: RouteStep[]
  destinations: RouteEnd[]
  live?: boolean
}>()
</script>

<template>
  <section
    class="grid items-center gap-4 rounded-xl border border-border bg-card px-4 py-4 md:grid-cols-[minmax(0,15rem)_minmax(0,1fr)_minmax(0,15rem)] md:px-5"
    aria-label="Route"
  >
    <div class="flex min-w-0 items-center gap-3">
      <TransferEngineMark :engine="source.engine" size="lg" />
      <div class="min-w-0">
        <div class="text-xs text-muted-foreground">{{ source.role }}</div>
        <div class="truncate text-sm font-semibold text-foreground" :title="source.title">{{ source.title || "Not set" }}</div>
        <div class="truncate text-xs text-muted-foreground" :title="source.detail">{{ source.detail }}</div>
      </div>
    </div>

    <div class="relative py-2" :class="live ? 'route-live' : ''">
      <div class="absolute left-0 right-0 top-[1.15rem] h-px bg-border md:block" aria-hidden="true">
        <div v-if="live" class="route-flow h-px" />
      </div>
      <ol class="relative flex justify-around gap-2">
        <li v-for="(s, i) in steps" :key="s.key" class="flex min-w-0 flex-col items-center gap-1 text-center">
          <span
            class="flex h-6 w-6 items-center justify-center rounded-full border text-xs font-semibold tabular-nums"
            :class="{
              'border-border bg-card text-muted-foreground': s.state === 'pending',
              'border-primary bg-primary text-primary-foreground': s.state === 'now',
              'border-emerald-500/60 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400': s.state === 'done',
              'border-red-500/60 bg-red-500/10 text-red-600 dark:text-red-400': s.state === 'failed',
            }"
          >
            <Check v-if="s.state === 'done'" class="h-3.5 w-3.5" :stroke-width="2.25" />
            <X v-else-if="s.state === 'failed'" class="h-3.5 w-3.5" :stroke-width="2.25" />
            <template v-else>{{ i + 1 }}</template>
          </span>
          <span class="text-xs font-medium text-foreground">{{ s.label }}</span>
          <span class="max-w-[10rem] truncate text-[11px] tabular-nums text-muted-foreground" :title="s.detail">{{ s.detail }}</span>
          <span class="sr-only">{{ { pending: "not started", now: "in progress", done: "done", failed: "failed" }[s.state] }}</span>
        </li>
      </ol>
    </div>

    <div class="flex min-w-0 flex-col gap-2">
      <div v-for="(d, i) in destinations" :key="i" class="flex min-w-0 items-center justify-end gap-3 text-right">
        <div class="min-w-0">
          <div class="text-xs text-muted-foreground">{{ d.role }}</div>
          <div class="truncate text-sm font-semibold text-foreground" :title="d.title">{{ d.title || "Not set" }}</div>
          <div class="truncate text-xs text-muted-foreground" :class="d.mono ? 'font-mono' : ''" :title="d.detail">{{ d.detail }}</div>
          <div v-if="d.overwrite" class="mt-0.5 inline-flex items-center gap-1 text-[11px] font-medium text-amber-600 dark:text-amber-400">
            <AlertTriangle class="h-3 w-3" :stroke-width="2" /> {{ d.overwrite }}
          </div>
        </div>
        <TransferEngineMark :engine="d.engine" size="lg" />
      </div>
    </div>
  </section>
</template>

<style scoped>
.route-flow {
  background: linear-gradient(90deg, transparent 0%, var(--primary) 45%, var(--primary) 55%, transparent 100%);
  background-size: 40% 100%;
  background-repeat: no-repeat;
  animation: route-flow 1.8s linear infinite;
}
@keyframes route-flow {
  from {
    background-position: -40% 0;
  }
  to {
    background-position: 140% 0;
  }
}
@media (prefers-reduced-motion: reduce) {
  .route-flow {
    animation: none;
    background: var(--primary);
  }
}
</style>
