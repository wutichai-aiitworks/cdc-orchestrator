<script setup lang="ts">
// Gephi-style topology (ADR 0010): force-directed graph of gateway → sites → databases in use.
// Node size follows connections; colour follows the site's status (its databases share it, so each
// site reads as a cluster); curved edges; live tunnels show moving traffic. Click a site to select it.
// Every node can be dragged (edges follow); the arrangement is remembered per browser.
import { RotateCcw } from "@lucide/vue"
import { placed } from "~/utils/dragNodes"
import { buildGraph, curvePath, forceLayout, GATEWAY_ID, nodeRadius, type GraphNode } from "~/utils/tunnelGraph"
import type { TopoTunnel } from "~/utils/tunnelTopology"
import type { Tone } from "~/utils/transferView"

const props = defineProps<{ tunnels: TopoTunnel[]; gatewayReachable: boolean }>()
const emit = defineEmits<{ select: [id: string] }>()

const box = ref<HTMLElement | null>(null)
const width = ref(800)
const height = computed(() => Math.max(520, Math.min(700, 420 + props.tunnels.length * 40)))
let ro: ResizeObserver | null = null
onMounted(() => {
  ro = new ResizeObserver(([e]) => (width.value = Math.round(e.contentRect.width)))
  if (box.value) ro.observe(box.value)
})
onBeforeUnmount(() => ro?.disconnect())

const graph = computed(() => buildGraph(props.tunnels, props.gatewayReachable))
// Keep clear of the toolbar (top right) and the legend (bottom).
const pos = computed(() => forceLayout(graph.value, width.value, height.value, 400, { top: 64, bottom: 40 }))
const drag = useDragNodes("cdc-topology-graph-layout")
const P = (id: string) => placed(pos.value.get(id) || { x: 0, y: 0 }, drag.off(id), width.value, height.value, 24)
const selectSite = drag.guardClick((n: GraphNode) => n.kind === "site" && emit("select", n.id))

const FILL: Record<Tone, string> = { ok: "#10b981", warn: "#f59e0b", err: "#ef4444", run: "#10b981", idle: "#a1a1aa" }
const fill = (n: GraphNode) => (n.kind === "gateway" ? "var(--primary)" : FILL[n.tone])
const hovered = ref<string | null>(null)
/** Hovering a node highlights it and its neighbours (Gephi's ego view); so does dragging one. */
const neighbours = computed(() => {
  const h = drag.dragging.value ?? hovered.value
  if (!h) return null
  const set = new Set([h])
  for (const e of graph.value.edges) {
    if (e.source === h) set.add(e.target)
    if (e.target === h) set.add(e.source)
  }
  return set
})
const dim = (id: string) => (neighbours.value && !neighbours.value.has(id) ? 0.2 : 1)
const STATE: Record<Tone, string> = { ok: "connected", warn: "no handshake yet", err: "disconnected", run: "connected", idle: "not on gateway" }
</script>

<template>
  <div ref="box" class="relative overflow-hidden rounded-xl border border-border bg-card" :style="{ height: `${height}px` }">
    <svg :width="width" :height="height" role="img" :aria-label="`Force-directed graph: gateway, ${tunnels.length} site(s) and their databases`">
      <!-- edges -->
      <g fill="none">
        <template v-for="(e, i) in graph.edges" :key="`e${i}`">
          <path
            :d="curvePath(P(e.source), P(e.target))"
            :stroke="FILL[e.tone]"
            :stroke-width="e.source === GATEWAY_ID ? 2 : 1.25"
            :stroke-dasharray="e.tone === 'ok' ? undefined : '5 5'"
            :opacity="0.45 * Math.min(dim(e.source), dim(e.target)) * (neighbours ? 2 : 1)"
          />
          <path v-if="e.live" class="graph-flow" :d="curvePath(P(e.source), P(e.target))" :stroke="FILL.ok" stroke-width="2.5" stroke-linecap="round" stroke-dasharray="3 20" :opacity="Math.min(dim(e.source), dim(e.target))" />
        </template>
      </g>
      <!-- nodes -->
      <g
        v-for="n in graph.nodes"
        :key="n.id"
        :transform="`translate(${P(n.id).x.toFixed(1)} ${P(n.id).y.toFixed(1)})`"
        :opacity="dim(n.id)"
        class="graph-node outline-none"
        :class="drag.dragging.value === n.id ? 'cursor-grabbing' : 'cursor-grab'"
        tabindex="0"
        :aria-label="`${n.label}: drag or use arrow keys to move${n.kind === 'site' ? '; click to select' : ''}`"
        v-bind="drag.handlers(n.id)"
        @mouseenter="hovered = n.id"
        @mouseleave="hovered = null"
        @click="selectSite(n)"
        @keydown.enter="n.kind === 'site' && emit('select', n.id)"
      >
        <title>{{ n.label }} — {{ n.sub }}{{ n.kind !== "db" ? ` (${n.kind === "gateway" ? (gatewayReachable ? "online" : "unreachable") : STATE[n.tone]})` : n.active ? `, ${n.active} open connection(s)` : "" }}</title>
        <circle :r="nodeRadius(n) + 5" :fill="fill(n)" :opacity="drag.dragging.value === n.id ? 0.3 : 0.15" />
        <circle class="graph-focus" :r="nodeRadius(n) + 8" fill="none" stroke="var(--ring)" stroke-width="2" />
        <circle :r="nodeRadius(n)" :fill="fill(n)" stroke="var(--card)" stroke-width="2" />
        <text v-if="n.kind === 'gateway'" text-anchor="middle" dy="0.35em" class="fill-[var(--primary-foreground)] text-[11px] font-semibold">GW</text>
        <!-- label with a solid backing so edges pass behind it -->
        <text
          :y="nodeRadius(n) + 15"
          text-anchor="middle"
          class="fill-[var(--foreground)] font-medium"
          :class="n.kind === 'db' ? 'text-[10px]' : 'text-[12px]'"
          paint-order="stroke"
          stroke="var(--card)"
          stroke-width="4"
          stroke-linejoin="round"
        >{{ n.label }}</text>
        <text
          :y="nodeRadius(n) + (n.kind === 'db' ? 27 : 29)"
          text-anchor="middle"
          class="fill-[var(--muted-foreground)] font-mono text-[10px]"
          paint-order="stroke"
          stroke="var(--card)"
          stroke-width="4"
          stroke-linejoin="round"
        >{{ n.sub }}</text>
      </g>
    </svg>

    <p v-if="!tunnels.length" class="absolute inset-x-0 bottom-6 text-center text-sm text-muted-foreground">No sites yet — create a tunnel to connect one.</p>
    <div class="absolute bottom-3 left-3 flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-muted-foreground">
      <span class="flex items-center gap-1"><span class="h-2 w-2 rounded-full bg-emerald-500" />Connected</span>
      <span class="flex items-center gap-1"><span class="h-2 w-2 rounded-full bg-amber-500" />No handshake</span>
      <span class="flex items-center gap-1"><span class="h-2 w-2 rounded-full bg-red-500" />Disconnected</span>
      <span>Size = connections · small nodes = databases in use · hover to focus · drag to arrange</span>
    </div>
    <button
      v-if="drag.moved.value"
      type="button"
      class="absolute bottom-2.5 right-3 flex items-center gap-1 rounded-md border border-border bg-card px-2 py-1 text-[11px] text-muted-foreground hover:text-foreground"
      @click="drag.reset"
    >
      <RotateCcw class="h-3 w-3" :stroke-width="2" />Reset layout
    </button>
  </div>
</template>

<style scoped>
.graph-node {
  touch-action: none; /* let touch drag the node instead of scrolling the page */
}
.graph-focus {
  opacity: 0;
}
.graph-node:focus-visible .graph-focus {
  opacity: 1;
}
.graph-flow {
  animation: graph-flow 1.6s linear infinite;
}
@keyframes graph-flow {
  to {
    stroke-dashoffset: -23;
  }
}
@media (prefers-reduced-motion: reduce) {
  .graph-flow {
    animation: none;
  }
}
</style>
