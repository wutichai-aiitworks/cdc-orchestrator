<script setup lang="ts">
// Source → steps → destination(s). The one distinctive element of a Copy & Sync page, shared by
// every tool. Flat, like the rest of the app: solid brass fills and 1px borders, no shadows or
// glows. The track fills up to the step a run reached. Motion (live flow, current-step ring) only
// while a run is live, and never under reduced motion.
import { AlertTriangle, ArrowRight, Check, X } from "@lucide/vue"
import type { RouteEnd } from "~/utils/transferTools/types"
import type { RouteStep } from "~/utils/transferView"

const props = defineProps<{
  source: RouteEnd
  steps: RouteStep[]
  destinations: RouteEnd[]
  live?: boolean
}>()

/** How far the gold track runs: to the centre of the last step the run reached (steps are evenly spaced). */
const reached = computed(() => {
  let last = -1
  props.steps.forEach((s, i) => {
    if (s.state !== "pending") last = i
  })
  return last
})
const progress = computed(() => {
  const n = props.steps.length
  if (!n || reached.value < 0) return 0
  // A finished run fills the whole track, through to the target.
  if (props.steps.every((s) => s.state === "done")) return 100
  return ((reached.value + 0.5) / n) * 100
})
const failed = computed(() => props.steps.some((s) => s.state === "failed"))
const doneCount = computed(() => props.steps.filter((s) => s.state === "done").length)
/** One-line status above the track in the stacked layouts: "2 of 3 done", "Running: Data", "Stopped at Data". */
const headline = computed(() => {
  const stop = props.steps.find((s) => s.state === "failed")
  if (stop) return `Stopped at ${stop.label}`
  const now = props.steps.find((s) => s.state === "now")
  if (now) return `${now.label} · step ${props.steps.indexOf(now) + 1} of ${props.steps.length}`
  return `${doneCount.value} of ${props.steps.length} done`
})
const STATE_TEXT = { pending: "not started", now: "in progress", done: "done", failed: "failed" } as const
</script>

<template>
  <section class="route-card relative overflow-hidden rounded-xl border border-border bg-card" :class="live ? 'is-live' : ''" aria-label="Route">
    <div class="route-grid grid items-center">
      <!-- Source -->
      <div class="route-source flex min-w-0 items-center gap-4">
        <span class="route-medallion"><TransferEngineMark :engine="source.engine" size="lg" /></span>
        <div class="min-w-0">
          <div class="route-role">{{ source.role }}</div>
          <div class="mt-1 truncate text-base font-semibold tracking-tight text-foreground" :title="source.title">{{ source.title || "Not set" }}</div>
          <div class="mt-0.5 truncate text-xs text-muted-foreground" :class="source.mono ? 'font-mono' : ''" :title="source.detail">{{ source.detail }}</div>
          <div v-if="source.overwrite" class="mt-1 inline-flex items-center gap-1 text-[11px] font-medium text-amber-600 dark:text-amber-400">
            <AlertTriangle class="h-3 w-3" :stroke-width="2" /> {{ source.overwrite }}
          </div>
        </div>
      </div>

      <!-- Source → target connector (stacked layouts only) -->
      <div class="route-arrow" aria-hidden="true">
        <span class="route-arrow-line" />
        <span class="route-arrow-head"><ArrowRight class="h-3.5 w-3.5" :stroke-width="2" /></span>
        <span class="route-arrow-line" />
      </div>

      <!-- Steps on a track -->
      <div class="route-steps">
        <div class="route-steps-head">
          <span class="route-role">Progress</span>
          <span class="truncate text-xs tabular-nums text-muted-foreground">{{ headline }}</span>
        </div>
        <div class="relative py-1">
          <div class="route-track absolute left-0 right-0 top-[1.125rem] h-[2px] rounded-full" aria-hidden="true">
            <div class="route-progress h-full rounded-full" :class="failed ? 'is-failed' : ''" :style="{ width: `${progress}%` }" />
            <div v-if="live" class="route-flow absolute inset-0 rounded-full" />
          </div>
          <ol class="relative flex justify-around gap-2">
            <li v-for="(s, i) in steps" :key="s.key" class="flex min-w-0 flex-1 flex-col items-center text-center">
              <span class="route-node" :class="`is-${s.state}`">
                <Check v-if="s.state === 'done'" class="h-4 w-4" :stroke-width="2.5" />
                <X v-else-if="s.state === 'failed'" class="h-4 w-4" :stroke-width="2.5" />
                <template v-else>{{ i + 1 }}</template>
              </span>
              <span class="mt-3 max-w-full truncate text-sm font-medium tracking-tight" :class="s.state === 'pending' ? 'text-muted-foreground' : 'text-foreground'" :title="s.label">{{ s.label }}</span>
              <span class="mt-0.5 max-w-full truncate text-[11px] tabular-nums text-muted-foreground" :title="s.detail">{{ s.detail }}</span>
              <span class="sr-only">{{ STATE_TEXT[s.state] }}</span>
            </li>
          </ol>
        </div>
      </div>

      <!-- Destination(s) -->
      <div class="route-dests flex min-w-0 flex-col gap-4">
        <div v-for="(d, i) in destinations" :key="i" class="route-dest flex min-w-0 items-center gap-4">
          <span class="route-medallion route-dest-mark"><TransferEngineMark :engine="d.engine" size="lg" /></span>
          <div class="route-dest-text min-w-0">
            <div class="route-role">{{ d.role }}</div>
            <div class="mt-1 truncate text-base font-semibold tracking-tight text-foreground" :title="d.title">{{ d.title || "Not set" }}</div>
            <div class="mt-0.5 truncate text-xs text-muted-foreground" :class="d.mono ? 'font-mono' : ''" :title="d.detail">{{ d.detail }}</div>
            <div v-if="d.overwrite" class="mt-1 inline-flex items-center gap-1 text-[11px] font-medium text-amber-600 dark:text-amber-400">
              <AlertTriangle class="h-3 w-3" :stroke-width="2" /> {{ d.overwrite }}
            </div>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
/* Brass tones derived from the app primary, so both themes and any future primary follow. */
.route-card {
  --brass: var(--primary);
  --brass-deep: color-mix(in oklab, var(--primary) 78%, black);
  --brass-line: color-mix(in oklab, var(--primary) 40%, var(--border));
  container-type: inline-size;
}

/* Layout follows the card's own width, not the viewport: on tool pages the sidebar and the
   saved-profiles rail take most of a laptop screen.
     narrow  (< 34rem): source, target, then the steps band — one column
     medium  (34–58rem): source → target on one row, the steps band underneath
     wide    (≥ 58rem): source | steps | target on one line, no band */
.route-grid {
  --pad: 1.25rem;
  grid-template-columns: minmax(0, 1fr);
  grid-template-areas: "source" "dest" "steps";
  gap: 1rem;
  padding: var(--pad) var(--pad) 0;
}
.route-source {
  grid-area: source;
}
.route-dests {
  grid-area: dest;
}
.route-arrow {
  grid-area: arrow;
  display: none;
}
.route-steps {
  grid-area: steps;
  /* Full-bleed band at the bottom of the card. */
  margin: 0.25rem calc(var(--pad) * -1) 0;
  padding: 0.875rem var(--pad) 1.125rem;
  border-top: 1px solid var(--border);
  background: color-mix(in oklab, var(--muted) 45%, var(--card));
}
.route-steps-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.75rem;
  margin-bottom: 0.875rem;
}
/* The node outline cuts the track; it must match the band behind it. */
.route-steps .route-node {
  outline-color: color-mix(in oklab, var(--muted) 45%, var(--card));
}

@container (min-width: 34rem) {
  .route-grid {
    --pad: 1.5rem;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    grid-template-areas: "source arrow dest" "steps steps steps";
    column-gap: 1.25rem;
  }
  .route-arrow {
    display: flex;
    align-items: center;
    width: clamp(3rem, 12cqi, 8rem);
  }
  .route-dest {
    justify-content: flex-end;
    text-align: right;
  }
  .route-dest-mark {
    order: 2;
  }
  .route-dest-text {
    order: 1;
  }
}

@container (min-width: 58rem) {
  .route-grid {
    --pad: 1.75rem;
    grid-template-columns: minmax(0, 16rem) minmax(0, 1fr) minmax(0, 16rem);
    grid-template-areas: "source steps dest";
    padding: var(--pad);
  }
  .route-arrow,
  .route-steps-head {
    display: none;
  }
  .route-steps {
    margin: 0;
    padding: 0;
    border: 0;
    background: none;
  }
  .route-steps .route-node {
    outline-color: var(--card);
  }
}

/* Source → target connector: hairline, brass arrow in a small tile. */
.route-arrow-line {
  flex: 1;
  height: 1px;
  background: var(--brass-line);
}
.route-arrow-head {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 1.75rem;
  height: 1.75rem;
  border-radius: 9999px;
  border: 1px solid var(--brass-line);
  color: var(--brass-deep);
  background: var(--card);
}
:global(.dark) .route-arrow-head {
  color: color-mix(in oklab, var(--brass) 85%, white);
}

/* Small-caps role label: SOURCE / TARGET / SFTP… */
.route-role {
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.18em;
  text-transform: uppercase;
  color: var(--brass-deep);
}
:global(.dark) .route-role {
  color: color-mix(in oklab, var(--brass) 85%, white);
}

/* Engine logo in a flat tile with a thin brass border. */
.route-medallion {
  display: inline-flex;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  width: 3.5rem;
  height: 3.5rem;
  border-radius: 0.75rem;
  border: 1px solid var(--brass-line);
  background: var(--card);
}

/* Engines without a logo (MongoDB, CSV, SFTP, file): brass lettering, no inner tile. */
.route-medallion :deep(span[role="img"]) {
  border: 0;
  background: transparent;
  color: var(--brass-deep);
  font-size: 0.875rem;
  letter-spacing: 0.04em;
}
:global(.dark) .route-medallion :deep(span[role="img"]) {
  color: color-mix(in oklab, var(--brass) 85%, white);
}

/* Track: hairline, filled with solid brass up to the reached step. */
.route-track {
  background: var(--border);
}
.route-progress {
  background: var(--brass);
  transition: width 600ms cubic-bezier(0.22, 1, 0.36, 1);
}
.route-progress.is-failed {
  background: linear-gradient(90deg, var(--brass) 0 calc(100% - 3rem), rgb(239 68 68) calc(100% - 3rem));
}

/* Step nodes. The card-coloured outline only cuts the track around each node (not a shadow). */
.route-node {
  position: relative;
  display: flex;
  width: 2.25rem;
  height: 2.25rem;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
  font-size: 0.8125rem;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  border: 1px solid var(--border);
  background: var(--card);
  color: var(--muted-foreground);
  outline: 5px solid var(--card);
  transition: background-color 300ms, border-color 300ms, color 300ms;
}
.route-node.is-done {
  border-color: var(--brass);
  background: var(--brass);
  color: var(--primary-foreground);
}
.route-node.is-now {
  border: 1.5px solid var(--brass);
  color: var(--brass-deep);
  background: color-mix(in oklab, var(--brass) 10%, var(--card));
}
:global(.dark) .route-node.is-now {
  color: color-mix(in oklab, var(--brass) 85%, white);
}
.is-live .route-node.is-now::after {
  content: "";
  position: absolute;
  inset: -6px;
  border-radius: inherit;
  border: 1px solid var(--brass);
  animation: route-ring 1.8s cubic-bezier(0.22, 1, 0.36, 1) infinite;
}
.route-node.is-failed {
  border-color: rgb(239 68 68);
  color: rgb(220 38 38);
  background: color-mix(in oklab, rgb(239 68 68) 8%, var(--card)); /* opaque: the track must not show through */
}

/* Live: a brighter segment travels along the track. */
.route-flow {
  background: linear-gradient(90deg, transparent 0%, color-mix(in oklab, var(--brass) 70%, white) 50%, transparent 100%);
  background-size: 30% 100%;
  background-repeat: no-repeat;
  animation: route-flow 2.2s cubic-bezier(0.45, 0, 0.55, 1) infinite;
}
@keyframes route-flow {
  from {
    background-position: -30% 0;
  }
  to {
    background-position: 130% 0;
  }
}
@keyframes route-ring {
  from {
    opacity: 0.7;
    transform: scale(0.85);
  }
  to {
    opacity: 0;
    transform: scale(1.35);
  }
}
@media (prefers-reduced-motion: reduce) {
  .route-flow {
    animation: none;
    background: color-mix(in oklab, var(--brass) 45%, transparent);
  }
  .is-live .route-node.is-now::after {
    animation: none;
    opacity: 0.5;
    transform: none;
  }
  .route-progress {
    transition: none;
  }
}
</style>
