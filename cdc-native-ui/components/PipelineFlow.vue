<script setup lang="ts">
import {
  Activity,
  CircleStop,
  Database,
  Pause,
  Server,
  Snowflake,
  TriangleAlert,
  Waves,
  Zap,
} from "@lucide/vue"

export type FlowNodeId = "source" | "transform" | "target" | "mapping" | "review"

const props = withDefaults(
  defineProps<{
    variant?: "welcome" | "designer" | "viewer"
    pattern?: "source_to_target" | "source_to_lake" | "source_to_stream" | "source_to_olake" | null
    status?: string | null
    activeNode?: FlowNodeId | null
    sourceConfigured?: boolean
    transformConfigured?: boolean
    targetConfigured?: boolean
    mappingConfigured?: boolean
    reviewReady?: boolean
    sourceCaption?: string
    transformCaption?: string
    targetCaption?: string
    mappingCaption?: string
    sourceLabel?: string
    targetLabel?: string
    sourceImage?: string
    targetImage?: string
  }>(),
  {
    variant: "welcome",
    pattern: "source_to_target",
    status: "unknown",
    activeNode: null,
    sourceConfigured: false,
    transformConfigured: false,
    targetConfigured: false,
    mappingConfigured: false,
    reviewReady: false,
    sourceCaption: "",
    transformCaption: "",
    targetCaption: "",
    mappingCaption: "",
    sourceLabel: "Source",
    targetLabel: "Destination",
    sourceImage: "",
    targetImage: "",
  },
)

const emit = defineEmits<{
  select: [node: FlowNodeId]
}>()

const isDesigner = computed(() => props.variant === "designer")
const isViewer = computed(() => props.variant === "viewer")
const showMapping = computed(() => props.pattern === "source_to_target" && isDesigner.value)

type ViewerState = "running" | "busy" | "paused" | "failed" | "idle"

const statusTokens = computed(() => new Set(
  String(props.status || "unknown")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .split(/\s+/)
    .filter(Boolean),
))
const hasStatus = (...values: string[]) => values.some((value) => statusTokens.value.has(value))
const viewerState = computed<ViewerState>(() => {
  if (hasStatus("failed", "error", "unhealthy", "dead", "down", "offline")) return "failed"
  if (hasStatus("starting", "restarting", "preparing", "pending", "rebalancing")) return "busy"
  if (hasStatus("paused", "pause")) return "paused"
  if (hasStatus("running", "healthy", "online", "live", "up")) return "running"
  return "idle"
})
const flowIsMoving = computed(() => viewerState.value === "running" || viewerState.value === "busy")
const flowLineTone = computed(() => {
  if (viewerState.value === "failed") return "text-red-500"
  if (viewerState.value === "paused" || viewerState.value === "busy") return "text-yellow-500"
  if (viewerState.value === "running") return "text-emerald-500"
  return "text-border"
})
const engineTone = computed(() => {
  if (viewerState.value === "failed") return "border-red-500/60 ring-red-500/20"
  if (viewerState.value === "paused" || viewerState.value === "busy") return "border-yellow-500/60 ring-yellow-500/20"
  if (viewerState.value === "running") return "border-emerald-500/60 ring-emerald-500/20"
  return "border-border ring-border/30"
})
const stateIcon = computed(() => {
  if (viewerState.value === "failed") return TriangleAlert
  if (viewerState.value === "paused") return Pause
  if (viewerState.value === "idle") return CircleStop
  return Activity
})
const targetIcon = computed(() => {
  if (props.pattern === "source_to_lake") return Snowflake
  if (props.pattern === "source_to_stream") return Waves
  if (props.pattern === "source_to_olake") return Zap
  return Server
})
const statusLabel = computed(() => String(props.status || "Unknown"))

type FlowNode = {
  id: FlowNodeId
  title: string
  image: string
  fallback: string
  glow: string
  configured: () => boolean
  caption: () => string
  idle: string
}

const coreNodes = computed<FlowNode[]>(() => [
  {
    id: "source",
    title: "Source",
    image: "/images/cdc/pipeline-source.png",
    fallback: "Add a source database",
    glow: "ring-emerald-400/80 shadow-[0_0_28px_rgba(16,185,129,0.28)]",
    configured: () => props.sourceConfigured,
    caption: () => props.sourceCaption,
    idle: "Read change events",
  },
  {
    id: "transform",
    title: "Transform",
    image: "/images/cdc/pipeline-transform.png",
    fallback: "Optional SMT",
    glow: "ring-brand-400/80",
    configured: () => props.transformConfigured,
    caption: () => props.transformCaption,
    idle: "Shape the events",
  },
  {
    id: "target",
    title: "Target",
    image: "/images/cdc/pipeline-target.png",
    fallback: "Add a destination",
    glow: "ring-teal-400/80",
    configured: () => props.targetConfigured,
    caption: () => props.targetCaption,
    idle: "Write change events",
  },
])

const extraNodes = computed<FlowNode[]>(() => {
  if (!showMapping.value) return []
  return [
    {
      id: "mapping",
      title: "Map tables",
      image: "/images/cdc/pipeline-mapping.png",
      fallback: "Map source → target",
      glow: "ring-violet-400/80",
      configured: () => props.mappingConfigured,
      caption: () => props.mappingCaption,
      idle: "Rename tables",
    },
    {
      id: "review",
      title: "Review",
      image: "/images/cdc/pipeline-review.png",
      fallback: "Confirm and save",
      glow: "ring-brand-400/60",
      configured: () => props.reviewReady,
      caption: () => "",
      idle: "Save pipeline",
    },
  ]
})

const nodes = computed<FlowNode[]>(() => [...coreNodes.value, ...extraNodes.value])

const isActive = (id: FlowNodeId) => props.activeNode === id

const nodeClass = (id: FlowNodeId, glow: string, configured: boolean) => {
  if (!isDesigner.value) return `bg-card ring-2 ${glow}`
  if (isActive(id)) return `bg-card ring-2 ${glow} scale-[1.03]`
  if (configured) return "bg-card ring-1 ring-border"
  return "bg-card/80 ring-1 ring-dashed ring-border hover:ring-primary/50"
}

const subtitle = (node: FlowNode) => {
  if (!isDesigner.value) return node.caption() || node.idle
  if (node.configured() && node.caption()) return node.caption()
  return node.fallback
}

const select = (id: FlowNodeId) => {
  if (isDesigner.value) emit("select", id)
}

const nodeSize = computed(() =>
  showMapping.value
    ? "w-[7.5rem] sm:w-[8.5rem]"
    : "w-[9.25rem] sm:w-44"
)

const imageSize = computed(() =>
  showMapping.value
    ? "h-16 w-16 sm:h-20 sm:w-20"
    : "h-24 w-24 sm:h-28 sm:w-28"
)
</script>

<template>
  <div v-if="isViewer" class="w-full overflow-x-auto">
    <div
      class="relative min-w-[700px] overflow-hidden rounded-2xl border border-border bg-card/70 px-6 py-7"
      :aria-label="`${sourceLabel} to ${targetLabel} pipeline is ${statusLabel}`"
      role="img"
    >
      <div
        class="pointer-events-none absolute inset-0 opacity-35"
        style="background-image: radial-gradient(rgb(var(--surface-600)) 1px, transparent 1px); background-size: 18px 18px"
      />

      <div class="relative grid grid-cols-[6rem_minmax(4rem,1fr)_6rem_minmax(4rem,1fr)_6rem] items-start">
        <div class="flex min-w-0 flex-col items-center">
          <div class="flex h-24 w-24 items-center justify-center rounded-2xl border border-border bg-background shadow-sm">
            <img v-if="sourceImage" :src="sourceImage" :alt="sourceLabel" class="h-12 w-12 object-contain" />
            <Database v-else class="h-10 w-10 text-primary" :stroke-width="1.5" />
          </div>
          <p class="mt-3 max-w-full truncate text-sm font-semibold text-foreground">{{ sourceLabel }}</p>
          <p class="max-w-full truncate text-xs text-muted-foreground">{{ sourceCaption || "Source database" }}</p>
        </div>

        <div class="relative z-10 h-24" aria-hidden="true">
          <svg
            viewBox="0 0 200 86"
            preserveAspectRatio="none"
            :class="['absolute inset-0 h-full w-full overflow-visible', flowLineTone]"
          >
            <path
              d="M 0 43 C 70 43, 115 25, 200 43"
              pathLength="100"
              :class="['flow-route', { 'flow-route-moving': flowIsMoving }]"
            />
            <path d="M 200 43 h 0.01" class="flow-node" />
          </svg>
          <span
            v-if="flowIsMoving"
            class="binary-packet binary-packet-left text-[#2F2F2F]"
          >010101</span>
        </div>

        <div class="flex flex-col items-center">
          <div
            :class="[
              'relative flex h-24 w-24 items-center justify-center rounded-2xl border-2 bg-white shadow-lg ring-4 transition-colors',
              engineTone,
            ]"
          >
            <img
              src="/favicon.png"
              alt="Hyper CDC"
              class="relative h-16 w-16 object-contain drop-shadow-lg"
              :class="{ 'animate-pulse motion-reduce:animate-none': flowIsMoving }"
            />
          </div>
          <p class="mt-3 text-base font-bold text-foreground">Hyper CDC</p>
          <p class="max-w-[11rem] truncate text-xs text-muted-foreground">{{ transformCaption || "Change data capture" }}</p>
          <span class="mt-2 inline-flex items-center gap-1.5 rounded-full border border-border bg-background/90 px-2.5 py-1 text-[11px] font-medium capitalize text-foreground">
            <component
              :is="stateIcon"
              class="h-3 w-3"
              :class="{ 'animate-spin motion-reduce:animate-none': viewerState === 'busy' }"
              :stroke-width="2"
            />
            {{ statusLabel }}
          </span>
        </div>

        <div class="relative h-24" aria-hidden="true">
          <svg
            viewBox="0 0 200 86"
            preserveAspectRatio="none"
            :class="['absolute inset-0 h-full w-full overflow-visible', flowLineTone]"
          >
            <path
              d="M 0 43 C 85 61, 130 43, 200 43"
              pathLength="100"
              :class="['flow-route', { 'flow-route-moving': flowIsMoving }]"
            />
            <path d="M 0 43 h 0.01" class="flow-node" />
          </svg>
          <span
            v-if="flowIsMoving"
            class="binary-packet binary-packet-right text-[#2F2F2F]"
          >010101</span>
        </div>

        <div class="flex min-w-0 flex-col items-center">
          <div class="flex h-24 w-24 items-center justify-center rounded-2xl border border-border bg-background shadow-sm">
            <img v-if="targetImage" :src="targetImage" :alt="targetLabel" class="h-12 w-12 object-contain" />
            <component v-else :is="targetIcon" class="h-10 w-10 text-primary" :stroke-width="1.5" />
          </div>
          <p class="mt-3 max-w-full truncate text-sm font-semibold text-foreground">{{ targetLabel }}</p>
          <p class="max-w-full truncate text-xs text-muted-foreground">{{ targetCaption || "Pipeline destination" }}</p>
        </div>
      </div>
    </div>
  </div>

  <div v-else class="w-full overflow-x-auto">
    <div class="relative min-w-max rounded-2xl">
      <div
        class="pointer-events-none absolute inset-0 rounded-2xl opacity-40"
        style="background-image: radial-gradient(rgb(var(--surface-600)) 1px, transparent 1px); background-size: 16px 16px"
      />

      <div class="relative flex items-center px-3 py-7 sm:px-6">
        <template v-for="(node, index) in nodes" :key="node.id">
          <component
            :is="isDesigner ? 'button' : 'div'"
            type="button"
            :class="[
              'relative z-10 flex shrink-0 flex-col items-center rounded-2xl p-3 transition-all duration-200',
              nodeSize,
              nodeClass(node.id, node.glow, node.configured()),
              isDesigner ? 'cursor-pointer' : '',
            ]"
            @click="select(node.id)"
          >
            <span
              v-if="node.configured()"
              class="absolute top-2 right-2 flex h-5 w-5 items-center justify-center rounded-full bg-emerald-500 text-white"
            >
              <svg class="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7" />
              </svg>
            </span>
            <div :class="['flex items-center justify-center overflow-hidden rounded-xl bg-white', imageSize]">
              <img :src="node.image" :alt="node.title" class="h-full w-full object-contain" />
            </div>
            <p class="mt-2 text-sm font-semibold text-foreground">{{ node.title }}</p>
            <p class="max-w-full truncate px-1 text-[11px] text-muted-foreground">{{ subtitle(node) }}</p>
          </component>

          <div
            v-if="index < nodes.length - 1"
            class="relative z-0 flex min-h-8 min-w-[2.5rem] flex-1 items-center px-0 sm:min-w-[3.5rem]"
            aria-hidden="true"
          >
            <span class="block h-[2px] w-full bg-primary" />
            <svg class="absolute right-0 h-3.5 w-3.5 shrink-0 text-primary" viewBox="0 0 12 12" fill="currentColor">
              <path d="M1.2 1.1a.75.75 0 0 1 .8-.08l8.5 4.4a.75.75 0 0 1 0 1.36l-8.5 4.4A.75.75 0 0 1 1 10.4V1.6a.75.75 0 0 1 .2-.5z" />
            </svg>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
@keyframes flow-dashes {
  to {
    stroke-dashoffset: -12;
  }
}

.flow-route {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linecap: round;
  stroke-dasharray: 1 6;
  opacity: 0.55;
  vector-effect: non-scaling-stroke;
}

.flow-route-moving {
  animation: flow-dashes 1.2s linear infinite;
}

.flow-node {
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  vector-effect: non-scaling-stroke;
  stroke-width: 7;
  opacity: 0.85;
}

@keyframes binary-left {
  0% {
    left: 0;
    top: 50%;
    opacity: 0;
  }
  15%,
  85% {
    opacity: 1;
  }
  50% {
    top: 29%;
  }
  100% {
    left: calc(100% - 2.25rem);
    top: 50%;
    opacity: 0;
  }
}

@keyframes binary-right {
  0% {
    left: 0;
    top: 50%;
    opacity: 0;
  }
  15%,
  85% {
    opacity: 1;
  }
  50% {
    top: 71%;
  }
  100% {
    left: calc(100% - 2.25rem);
    top: 50%;
    opacity: 0;
  }
}

.binary-packet {
  position: absolute;
  z-index: 1;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  font-size: 0.625rem;
  font-weight: 700;
  line-height: 1;
  letter-spacing: 0.04em;
  transform: translateY(-50%);
  white-space: nowrap;
}

.binary-packet-left {
  animation: binary-left 2.1s linear infinite;
}

.binary-packet-right {
  animation: binary-right 2.1s -0.7s linear infinite;
}

@media (prefers-reduced-motion: reduce) {
  .flow-route-moving {
    animation: none;
  }

  .binary-packet {
    display: none;
  }
}
</style>
