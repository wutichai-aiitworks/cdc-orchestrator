<script setup lang="ts">
// WireGuard topology (ADR 0010): Hyper CDC gateway in the centre, sites around it, and the
// databases used through each tunnel behind their site. Connected tunnels show moving traffic;
// others are dashed. Clicking a site selects it (the page scrolls to it and opens its readiness).
// The gateway, sites and database lists can be dragged (a site's list moves with it); the
// arrangement is remembered per browser.
import { Database, RotateCcw, Server, Shuffle } from "@lucide/vue"
import { placed } from "~/utils/dragNodes"
import { layoutTopology, type TopoTunnel } from "~/utils/tunnelTopology"
import type { Tone } from "~/utils/transferView"

const props = defineProps<{ tunnels: TopoTunnel[]; gatewayReachable: boolean }>()
const emit = defineEmits<{ select: [id: string] }>()

const box = ref<HTMLElement | null>(null)
const width = ref(800)
const height = computed(() => Math.max(520, Math.min(660, 460 + props.tunnels.length * 20)))
let ro: ResizeObserver | null = null
onMounted(() => {
  ro = new ResizeObserver(([e]) => (width.value = e.contentRect.width))
  if (box.value) ro.observe(box.value)
})
onBeforeUnmount(() => ro?.disconnect())

const layout = computed(() => layoutTopology(props.tunnels, width.value, height.value))
const drag = useDragNodes("cdc-topology-mesh-layout")
const GW = "gateway"
const gw = computed(() => placed({ x: layout.value.cx, y: layout.value.cy }, drag.off(GW), width.value, height.value, 40))
/** Sites at their dragged positions (x, y replaced; everything else from the layout). */
const sites = computed(() => layout.value.sites.map((s) => ({ ...s, ...placed(s, drag.off(s.id), width.value, height.value, 30) })))
const siteById = computed(() => Object.fromEntries(sites.value.map((s) => [s.id, s])))
/** A database list follows its site's drag, plus its own. */
const groups = computed(() =>
  layout.value.groups.map((g) => {
    const auto = layout.value.sites.find((s) => s.id === g.siteId)!
    const site = siteById.value[g.siteId]
    const base = { x: g.x + site.x - auto.x, y: g.y + site.y - auto.y }
    return { ...g, ...placed(base, drag.off(`db:${g.siteId}`), width.value, height.value, 8) }
  })
)
const selectSite = drag.guardClick((id: string) => emit("select", id))

const LINE: Record<Tone, string> = {
  ok: "var(--topo-ok)",
  warn: "var(--topo-warn)",
  err: "var(--topo-err)",
  run: "var(--topo-ok)",
  idle: "var(--topo-idle)",
}
const DOT: Record<Tone, string> = { ok: "bg-emerald-500", warn: "bg-amber-500", err: "bg-red-500", run: "bg-emerald-500", idle: "bg-zinc-400" }
const STATE: Record<Tone, string> = { ok: "Connected", warn: "No handshake yet", err: "Disconnected", run: "Connected", idle: "Not on gateway" }
</script>

<template>
  <div ref="box" class="topo relative overflow-hidden rounded-xl border border-border bg-card" :style="{ height: `${height}px` }" role="img" :aria-label="`WireGuard topology: gateway and ${tunnels.length} site(s)`">
    <!-- soft focus behind the gateway (flat, no shadows) -->
    <div class="pointer-events-none absolute inset-0" :style="{ background: `radial-gradient(circle at ${gw.x}px ${gw.y}px, color-mix(in oklab, var(--primary) 9%, transparent), transparent ${Math.round(layout.r1 * 1.25)}px)`}" />
    <svg class="absolute inset-0" :width="width" :height="height" aria-hidden="true">
      <!-- guide rings -->
      <circle :cx="gw.x" :cy="gw.y" :r="layout.r1" fill="none" stroke="var(--border)" stroke-dasharray="2 6" />
      <!-- gateway ↔ site -->
      <g v-for="s in sites" :key="`l-${s.id}`">
        <line :x1="gw.x" :y1="gw.y" :x2="s.x" :y2="s.y" :stroke="LINE[s.tone]" stroke-width="1.5" :stroke-dasharray="s.tone === 'ok' ? undefined : '5 5'" :opacity="s.tone === 'ok' ? 0.55 : 0.8" />
        <line v-if="s.tone === 'ok'" class="topo-flow" :x1="gw.x" :y1="gw.y" :x2="s.x" :y2="s.y" :stroke="LINE.ok" stroke-width="2.5" stroke-linecap="round" stroke-dasharray="3 22" />
      </g>
      <!-- site ↔ its databases (one connector to the list) -->
      <line
        v-for="g in groups"
        :key="`g-${g.siteId}`"
        :x1="siteById[g.siteId].x"
        :y1="siteById[g.siteId].y"
        :x2="g.x"
        :y2="g.y"
        stroke="var(--topo-idle)"
        stroke-width="1"
        :class="g.items.some((d) => d.active) ? 'topo-flow' : ''"
        :stroke-dasharray="g.items.some((d) => d.active) ? '3 14' : '2 4'"
      />
    </svg>

    <!-- gateway -->
    <div
      class="topo-drag absolute flex -translate-x-1/2 -translate-y-1/2 flex-col items-center text-center outline-none focus-visible:rounded-xl focus-visible:ring-2 focus-visible:ring-ring"
      :class="drag.dragging.value === GW ? 'z-20 cursor-grabbing' : 'cursor-grab'"
      :style="{ left: `${gw.x}px`, top: `${gw.y}px` }"
      tabindex="0"
      aria-label="Hyper CDC gateway: drag or use arrow keys to move"
      v-bind="drag.handlers(GW)"
    >
      <span class="relative flex h-16 w-16 items-center justify-center rounded-full bg-primary text-primary-foreground ring-8 ring-card">
        <Shuffle class="h-7 w-7" :stroke-width="1.75" />
        <span class="absolute bottom-0.5 right-0.5 h-3.5 w-3.5 rounded-full ring-2 ring-card" :class="gatewayReachable ? 'bg-emerald-500' : 'bg-red-500'" />
      </span>
      <span class="mt-2 rounded-md bg-card px-1.5 text-sm font-semibold text-foreground">Hyper CDC gateway</span>
      <span class="rounded-md bg-card px-1.5 font-mono text-[11px] text-muted-foreground">cdc-wireguard{{ gatewayReachable ? "" : " · unreachable" }}</span>
    </div>

    <!-- sites -->
    <button
      v-for="s in sites"
      :key="s.id"
      type="button"
      class="topo-drag group absolute flex w-44 -translate-x-1/2 items-center text-center focus:outline-none"
      :class="[s.labelAbove ? 'flex-col-reverse' : 'flex-col', drag.dragging.value === s.id ? 'z-20 cursor-grabbing' : 'cursor-grab']"
      :style="{ left: `${s.x}px`, top: `${s.y}px`, transform: `translate(-50%, ${s.labelAbove ? 'calc(-100% + 24px)' : '-24px'})` }"
      :title="`${s.label} — ${STATE[s.tone]}. Click for readiness; drag to move.`"
      v-bind="drag.handlers(s.id)"
      @click="selectSite(s.id)"
    >
      <span class="relative flex h-12 w-12 items-center justify-center rounded-xl border border-border bg-card text-muted-foreground transition-colors group-hover:border-primary group-hover:text-primary group-focus-visible:ring-2 group-focus-visible:ring-ring">
        <Server class="h-5 w-5" :stroke-width="1.75" />
        <span class="absolute -bottom-1 -right-1 h-3 w-3 rounded-full ring-2 ring-card" :class="DOT[s.tone]" />
      </span>
      <span class="flex max-w-full flex-col items-center rounded-md bg-card px-1.5 py-0.5" :class="s.labelAbove ? 'mb-1.5' : 'mt-1.5'">
      <span class="max-w-full truncate text-xs font-medium text-foreground">{{ s.label }}</span>
      <span class="max-w-full truncate font-mono text-[10px] text-muted-foreground">{{ s.sub }}</span>
      <span class="text-[10px]" :class="s.tone === 'ok' ? 'text-emerald-600 dark:text-emerald-400' : s.tone === 'err' ? 'text-red-600 dark:text-red-400' : s.tone === 'warn' ? 'text-amber-600 dark:text-amber-400' : 'text-muted-foreground'">{{ STATE[s.tone] }}</span>
      </span>
    </button>

    <!-- databases in use: a list beside each site -->
    <div
      v-for="g in groups"
      :key="`list-${g.siteId}`"
      class="topo-drag absolute flex -translate-y-1/2 flex-col gap-1 outline-none focus-visible:rounded-lg focus-visible:ring-2 focus-visible:ring-ring"
      :class="[g.side === 'left' ? '-translate-x-full items-end' : 'items-start', drag.dragging.value === `db:${g.siteId}` ? 'z-20 cursor-grabbing' : 'cursor-grab']"
      tabindex="0"
      :aria-label="`Databases used via ${siteById[g.siteId].label}: drag or use arrow keys to move`"
      v-bind="drag.handlers(`db:${g.siteId}`)"
      :style="{ left: `${g.x}px`, top: `${g.y}px` }"
    >
      <div
        v-for="d in g.items"
        :key="d.key"
        class="flex items-center gap-1.5 rounded-full border border-border bg-card px-2 py-0.5"
        :title="`${d.engine} at ${d.label} via ${siteById[g.siteId].label}${d.active ? ` — ${d.active} open connection(s)` : ''}`"
      >
        <Database class="h-3 w-3 shrink-0 text-muted-foreground" :stroke-width="2" />
        <span class="whitespace-nowrap text-[10px] font-medium text-foreground">{{ d.engine }}</span>
        <span class="whitespace-nowrap font-mono text-[10px] text-muted-foreground">{{ d.label }}</span>
        <span v-if="d.active" class="h-1.5 w-1.5 rounded-full bg-emerald-500" />
      </div>
    </div>

    <p v-if="!tunnels.length" class="absolute inset-x-0 bottom-6 text-center text-sm text-muted-foreground">No sites yet — create a tunnel to connect one.</p>

    <div class="absolute bottom-3 left-3 flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-muted-foreground">
      <span class="flex items-center gap-1"><span class="h-2 w-2 rounded-full bg-emerald-500" />Connected</span>
      <span class="flex items-center gap-1"><span class="h-2 w-2 rounded-full bg-amber-500" />No handshake</span>
      <span class="flex items-center gap-1"><span class="h-2 w-2 rounded-full bg-red-500" />Disconnected</span>
      <span class="flex items-center gap-1"><Database class="h-2.5 w-2.5" :stroke-width="2" />Database in use</span>
      <span>· drag to arrange</span>
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
.topo {
  --topo-ok: rgb(16 185 129);
  --topo-warn: rgb(245 158 11);
  --topo-err: rgb(239 68 68);
  --topo-idle: color-mix(in oklab, var(--muted-foreground) 55%, transparent);
}
.topo-drag {
  touch-action: none; /* let touch drag the node instead of scrolling the page */
  user-select: none;
}
.topo-flow {
  animation: topo-flow 1.6s linear infinite;
}
@keyframes topo-flow {
  to {
    stroke-dashoffset: -25;
  }
}
@media (prefers-reduced-motion: reduce) {
  .topo-flow {
    animation: none;
  }
}
</style>
