<script setup lang="ts">
// One run, expanded: what happened per step, the problem and what to do, tool extras (default
// slot, e.g. a dump file and its uploads), and the log.
import { Check, Square, Trash2, X } from "@lucide/vue"
import { formatDuration, isActive, jobDurationMs, startedLabel, type BaseJob, type Problem, type RouteStep } from "~/utils/transferView"

const props = defineProps<{
  job: BaseJob
  steps: RouteStep[]
  problems: Problem[]
  log: string
  continuous?: boolean
  stopVerb: string
  canDelete?: boolean
  now: number
}>()
const emit = defineEmits<{ stop: []; delete: [] }>()
const active = computed(() => isActive(props.job.status))
</script>

<template>
  <div class="space-y-4">
    <ol class="grid gap-3" :class="steps.length > 3 ? 'sm:grid-cols-4' : 'sm:grid-cols-3'">
      <li v-for="s in steps" :key="s.key" class="rounded-lg border border-border p-3">
        <div class="flex items-center gap-2 text-sm font-medium text-foreground">
          <span
            class="flex h-4 w-4 items-center justify-center rounded-full"
            :class="{
              'bg-emerald-500/15 text-emerald-600 dark:text-emerald-400': s.state === 'done',
              'bg-red-500/15 text-red-600 dark:text-red-400': s.state === 'failed',
              'bg-primary/20': s.state === 'now',
              'bg-muted': s.state === 'pending',
            }"
          >
            <Check v-if="s.state === 'done'" class="h-3 w-3" :stroke-width="2.5" />
            <X v-else-if="s.state === 'failed'" class="h-3 w-3" :stroke-width="2.5" />
            <span v-else-if="s.state === 'now'" class="h-1.5 w-1.5 animate-pulse rounded-full bg-primary motion-reduce:animate-none" />
          </span>
          {{ s.label }}
        </div>
        <div class="mt-1 text-xs tabular-nums text-muted-foreground">
          {{ s.state === "pending" ? "Not reached" : s.state === "now" ? (continuous && s.key === "live" ? "Live" : "In progress") : s.state === "failed" ? "Stopped here" : s.detail || "Done" }}
        </div>
      </li>
    </ol>

    <div v-for="(p, i) in problems" :key="i" role="alert" class="rounded-lg border border-red-500/30 bg-red-500/5 p-3 text-sm">
      <div class="font-medium text-foreground">{{ p.title }}</div>
      <div v-if="p.fix" class="mt-1 text-muted-foreground">{{ p.fix }}</div>
      <div v-if="p.detail" class="mt-2 whitespace-pre-wrap break-all font-mono text-xs text-red-600 dark:text-red-400">{{ p.detail }}</div>
    </div>

    <dl class="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-[8rem_minmax(0,1fr)]">
      <dt class="text-muted-foreground">Started</dt>
      <dd class="tabular-nums">
        {{ startedLabel(job.started_at || job.created_at, now) }}, {{ active ? "running for" : "took" }} {{ formatDuration(jobDurationMs(job, now)) }}
      </dd>
      <template v-if="job.exit_code != null && !active">
        <dt class="text-muted-foreground">Exit code</dt>
        <dd class="tabular-nums">{{ job.exit_code }}</dd>
      </template>
    </dl>

    <slot />

    <TransferLog :text="log" :live="active" title="Log" />

    <div class="flex flex-wrap items-center gap-2">
      <slot name="actions" />
      <AppButton v-if="active" size="sm" @click="emit('stop')"><Square class="h-3.5 w-3.5" :stroke-width="1.75" />{{ stopVerb }}</AppButton>
      <AppButton v-else-if="canDelete" size="sm" variant="ghost" @click="emit('delete')"><Trash2 class="h-3.5 w-3.5" :stroke-width="1.75" />Delete run</AppButton>
    </div>
  </div>
</template>
