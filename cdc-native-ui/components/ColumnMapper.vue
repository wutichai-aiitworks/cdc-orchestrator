<script setup lang="ts">
import { Key, X, Zap, RotateCcw, ChevronDown, ChevronUp } from "@lucide/vue"
import type { SourceTableColumn, ColumnLink } from "~/composables/useProfilesM3"
import {
  TRANSFORM_FUNCTIONS,
  TRANSFORM_BY_ID,
  TRANSFORM_CATEGORIES,
} from "~/utils/transformFunctions"

// ── props / emits ─────────────────────────────────────────────────────────
const props = withDefaults(
  defineProps<{
    sourceColumns: SourceTableColumn[]
    targetColumns: SourceTableColumn[]   // empty → auto-derive from source
    modelValue: ColumnLink[]
    loading?: boolean
    error?: string
  }>(),
  { loading: false, error: "" }
)

const emit = defineEmits<{
  "update:modelValue": [links: ColumnLink[]]
  retry: []
}>()

// ── layout constants ──────────────────────────────────────────────────────
const ROW_H = 44          // height of each column row in px
const HEADER_H = 40       // header row height
const FOOTER_H = 4        // bottom padding

// ── container ref + measured width ───────────────────────────────────────
const containerEl = ref<HTMLDivElement>()
const containerW = ref(600)

const ro = new ResizeObserver(([e]) => {
  containerW.value = e.contentRect.width
})
onMounted(() => {
  if (containerEl.value) {
    ro.observe(containerEl.value)
    containerW.value = containerEl.value.offsetWidth
  }
})
onUnmounted(() => ro.disconnect())

// ── effective target columns ──────────────────────────────────────────────
// Use catalog columns if available; otherwise derive names from source
const effectiveTargetCols = computed<{ name: string; data_type: string; nullable?: boolean; primary_key?: boolean }[]>(() => {
  if (props.targetColumns.length) return props.targetColumns
  return props.sourceColumns.map((c) => ({ name: c.name, data_type: c.data_type, nullable: c.nullable, primary_key: c.primary_key }))
})

// ── SVG sizing ────────────────────────────────────────────────────────────
const MIN_CANVAS_H = 200   // always show at least this much canvas height
const svgH = computed(
  () => Math.max(MIN_CANVAS_H, HEADER_H + Math.max(props.sourceColumns.length, effectiveTargetCols.value.length) * ROW_H + FOOTER_H)
)

// Horizontal split: source columns occupy 45%, target 45%, SVG gap in the middle 10%
const srcW = computed(() => containerW.value * 0.44)
const tgtX = computed(() => containerW.value * 0.56)

// Port positions
const SRC_PORT_X = computed(() => srcW.value + 4)
const TGT_PORT_X = computed(() => tgtX.value - 4)
const portY = (idx: number) => HEADER_H + idx * ROW_H + ROW_H / 2

const srcIdx = (name: string) => props.sourceColumns.findIndex((c) => c.name === name)
const tgtIdx = (name: string) => effectiveTargetCols.value.findIndex((c) => c.name === name)

// ── link helpers ──────────────────────────────────────────────────────────
const links = computed(() => props.modelValue)

const genId = () => `lnk-${Math.random().toString(36).slice(2, 9)}`

const emitLinks = (next: ColumnLink[]) => emit("update:modelValue", next)

const isSourceLinked = (name: string) => links.value.some((l) => l.sources.includes(name))
const isTargetLinked = (name: string) => links.value.some((l) => l.target === name)
const linksForTarget = (name: string) => links.value.filter((l) => l.target === name)

// ── path generation ───────────────────────────────────────────────────────
// Smooth S-curve bezier from source port to target port
const bezier = (sx: number, sy: number, tx: number, ty: number) => {
  const cx = (sx + tx) / 2
  return `M ${sx} ${sy} C ${cx} ${sy} ${cx} ${ty} ${tx} ${ty}`
}

// Build all bezier paths for a link (one per source column)
const linkPaths = (link: ColumnLink) => {
  const ti = tgtIdx(link.target)
  if (ti < 0) return []
  const ty = portY(ti)
  const tx = TGT_PORT_X.value
  return link.sources
    .map((src) => {
      const si = srcIdx(src)
      if (si < 0) return null
      return bezier(SRC_PORT_X.value, portY(si), tx, ty)
    })
    .filter((p): p is string => p !== null)
}

// Midpoint of a single bezier (for label placement)
const bezierMid = (sx: number, sy: number, tx: number, ty: number) => ({
  x: (sx + tx) / 2,
  y: (sy + ty) / 2,
})

const linkLabelPos = (link: ColumnLink) => {
  const ti = tgtIdx(link.target)
  if (ti < 0 || !link.sources.length) return null
  const si = srcIdx(link.sources[0])
  if (si < 0) return null
  return bezierMid(SRC_PORT_X.value, portY(si), TGT_PORT_X.value, portY(ti))
}

// ── select all / clear ────────────────────────────────────────────────────
const selectAll = () => {
  const linked = new Set(links.value.flatMap((l) => l.sources))
  const next = [...links.value]
  for (const col of props.sourceColumns) {
    if (linked.has(col.name)) continue
    const tgt = effectiveTargetCols.value.find((t) => t.name === col.name)
    next.push({
      id: genId(),
      sources: [col.name],
      target: tgt?.name ?? col.name,
      fn: null,
      fn_args: {},
    })
  }
  emitLinks(next)
}

const clearAll = () => {
  selectedLinkId.value = null
  emitLinks([])
}

// ── drag state ────────────────────────────────────────────────────────────
type Drag = {
  sourceName: string
  sx: number
  sy: number
  mx: number
  my: number
}
const drag = ref<Drag | null>(null)

const startDrag = (name: string, e: MouseEvent) => {
  if (!containerEl.value) return
  const rect = containerEl.value.getBoundingClientRect()
  const si = srcIdx(name)
  drag.value = {
    sourceName: name,
    sx: SRC_PORT_X.value,
    sy: portY(si),
    mx: e.clientX - rect.left,
    my: e.clientY - rect.top,
  }
}

const onMouseMove = (e: MouseEvent) => {
  if (!drag.value || !containerEl.value) return
  const rect = containerEl.value.getBoundingClientRect()
  drag.value = { ...drag.value, mx: e.clientX - rect.left, my: e.clientY - rect.top }
}

const cancelDrag = () => {
  drag.value = null
}

const dropOnTarget = (targetName: string) => {
  if (!drag.value) return
  const sourceName = drag.value.sourceName
  drag.value = null
  // Does a link to this target already exist?
  const existing = links.value.find((l) => l.target === targetName)
  if (existing) {
    if (!existing.sources.includes(sourceName)) {
      const fn = TRANSFORM_BY_ID.get(existing.fn ?? "")
      const maxSrc = fn?.maxSources ?? 1
      if (existing.sources.length < maxSrc) {
        emitLinks(
          links.value.map((l) =>
            l.id === existing.id ? { ...l, sources: [...l.sources, sourceName] } : l
          )
        )
      }
    }
  } else {
    emitLinks([
      ...links.value,
      { id: genId(), sources: [sourceName], target: targetName, fn: null, fn_args: {} },
    ])
  }
}

const dragPath = computed(() => {
  if (!drag.value) return ""
  const { sx, sy, mx, my } = drag.value
  const cx = (sx + mx) / 2
  return `M ${sx} ${sy} C ${cx} ${sy} ${cx} ${my} ${mx} ${my}`
})

// ── link selection / function picker ─────────────────────────────────────
const selectedLinkId = ref<string | null>(null)
const selectedLink = computed(() => links.value.find((l) => l.id === selectedLinkId.value) ?? null)
const pickerFnCategory = ref(TRANSFORM_CATEGORIES[0])

const selectLink = (id: string, e: MouseEvent) => {
  e.stopPropagation()
  selectedLinkId.value = id === selectedLinkId.value ? null : id
}

const removeLink = (id: string) => {
  if (selectedLinkId.value === id) selectedLinkId.value = null
  emitLinks(links.value.filter((l) => l.id !== id))
}

const removeSrcFromLink = (linkId: string, srcName: string) => {
  const link = links.value.find((l) => l.id === linkId)
  if (!link) return
  if (link.sources.length <= 1) {
    removeLink(linkId)
  } else {
    emitLinks(links.value.map((l) =>
      l.id === linkId ? { ...l, sources: l.sources.filter((s) => s !== srcName) } : l
    ))
  }
}

const setFn = (fnId: string | null) => {
  if (!selectedLinkId.value) return
  emitLinks(
    links.value.map((l) =>
      l.id === selectedLinkId.value ? { ...l, fn: fnId, fn_args: {} } : l
    )
  )
}

const setArg = (key: string, value: unknown) => {
  if (!selectedLinkId.value) return
  emitLinks(
    links.value.map((l) =>
      l.id === selectedLinkId.value
        ? { ...l, fn_args: { ...l.fn_args, [key]: value } }
        : l
    )
  )
}

const linkFnDef = computed(() => (selectedLink.value?.fn ? TRANSFORM_BY_ID.get(selectedLink.value.fn) : null) ?? null)

// Close picker when clicking outside
const onContainerClick = () => {
  selectedLinkId.value = null
}

// ── line color by state ───────────────────────────────────────────────────
const lineColor = (link: ColumnLink) => {
  if (link.id === selectedLinkId.value) return "stroke-brand-500"
  if (link.fn) return "stroke-violet-400"
  return "stroke-emerald-400/80"
}

const lineLabel = (link: ColumnLink) => {
  if (!link.fn) return null
  return TRANSFORM_BY_ID.get(link.fn)?.label ?? link.fn
}

// ── type badge color ──────────────────────────────────────────────────────
const typeBadge = (dt: string) => {
  const t = dt.toLowerCase()
  if (t.includes("int") || t.includes("numeric") || t.includes("decimal") || t.includes("float") || t.includes("double") || t.includes("real"))
    return "badge-neutral text-[10px]"
  if (t.includes("char") || t.includes("text") || t.includes("clob"))
    return "badge-neutral text-[10px] !bg-blue-500/10 !text-blue-400"
  if (t.includes("date") || t.includes("time") || t.includes("timestamp"))
    return "badge-neutral text-[10px] !bg-violet-500/10 !text-violet-400"
  if (t.includes("bool"))
    return "badge-neutral text-[10px] !bg-emerald-500/10 !text-emerald-400"
  return "badge-neutral text-[10px]"
}
</script>

<template>
  <div class="space-y-3">
    <!-- Toolbar -->
    <div class="flex flex-nowrap items-center justify-between gap-2">
      <div class="flex items-center gap-2">
        <span class="text-xs font-medium text-muted-foreground">
          <template v-if="loading">
            <span class="inline-flex items-center gap-1.5">
              <span class="h-3 w-3 animate-spin rounded-full border-2 border-current border-t-transparent opacity-60" />
              Loading columns…
            </span>
          </template>
          <template v-else-if="error">
            <span class="text-red-400">{{ error }}</span>
          </template>
          <template v-else>
            {{ sourceColumns.length }} source column{{ sourceColumns.length === 1 ? "" : "s" }}
            <template v-if="targetColumns.length"> · {{ targetColumns.length }} target columns</template>
          </template>
        </span>
      </div>
      <div class="flex shrink-0 gap-2">
        <button
          v-if="error"
          type="button"
          class="btn-secondary !h-8 !min-h-8 !whitespace-nowrap !px-3 !text-xs"
          @click="emit('retry')"
        >
          <RotateCcw class="h-3.5 w-3.5" :stroke-width="1.75" />
          Retry
        </button>
        <button
          v-else
          type="button"
          class="btn-secondary !h-8 !min-h-8 !whitespace-nowrap !px-3 !text-xs"
          :disabled="loading"
          @click="selectAll"
        >
          Select all
        </button>
        <button
          type="button"
          class="btn-secondary !h-8 !min-h-8 !whitespace-nowrap !px-3 !text-xs"
          :disabled="!links.length"
          @click="clearAll"
        >
          Clear
        </button>
      </div>
    </div>

    <!-- Mapper canvas -->
    <div
      ref="containerEl"
      class="relative select-none overflow-hidden rounded-xl border border-border bg-muted/30"
      :style="{ height: `${svgH}px` }"
      @click="onContainerClick"
      @mousemove="onMouseMove"
      @mouseup="cancelDrag"
      @mouseleave="cancelDrag"
    >
      <!-- Loading overlay -->
      <div
        v-if="loading"
        class="absolute inset-0 z-20 flex flex-col items-center justify-center gap-3 bg-surface-950/70"
      >
        <span class="h-8 w-8 animate-spin rounded-full border-[3px] border-brand-500/30 border-t-brand-500" />
        <p class="text-sm text-surface-300">Fetching columns…</p>
      </div>

      <!-- Error overlay (columns failed to load) -->
      <div
        v-else-if="error && !sourceColumns.length"
        class="absolute inset-0 z-20 flex flex-col items-center justify-center gap-3 px-6 text-center"
      >
        <div class="rounded-xl border border-red-500/30 bg-red-500/10 px-5 py-4 space-y-2 max-w-sm">
          <p class="text-sm font-semibold text-red-400">Could not load columns</p>
          <p class="text-xs text-surface-400">{{ error }}</p>
          <button
            type="button"
            class="btn-secondary !h-8 !min-h-8 !px-3 !text-xs mt-1"
            @click="emit('retry')"
          >
            <RotateCcw class="h-3.5 w-3.5" :stroke-width="1.75" />
            Retry
          </button>
        </div>
      </div>

      <!-- Empty state (columns loaded but empty list) -->
      <div
        v-else-if="!loading && !error && !sourceColumns.length"
        class="absolute inset-0 z-10 flex flex-col items-center justify-center gap-2 text-center px-6"
      >
        <p class="text-sm text-surface-400">No source columns available.</p>
        <p class="text-xs text-surface-500">Check that the table exists and your connection details are correct, then retry.</p>
      </div>
      <!-- Source panel -->
      <div
        class="absolute left-0 top-0 flex flex-col"
        :style="{ width: `${srcW}px`, height: `${svgH}px` }"
      >
        <!-- Header -->
        <div class="flex h-10 items-center px-3 text-xs font-semibold text-muted-foreground border-b border-border">
          Source columns
        </div>
        <!-- Column rows -->
        <div
          v-for="(col, idx) in sourceColumns"
          :key="col.name"
          class="group flex items-center gap-2 px-3 transition-colors"
          :class="[
            isSourceLinked(col.name) ? 'bg-emerald-500/5' : 'hover:bg-muted/60',
          ]"
          :style="{ height: `${ROW_H}px` }"
        >
          <Key v-if="col.primary_key" class="h-3 w-3 shrink-0 text-brand-500" :stroke-width="2" />
          <span v-else class="h-3 w-3 shrink-0" />
          <span class="min-w-0 flex-1 truncate text-sm font-medium">{{ col.name }}</span>
          <span :class="typeBadge(col.data_type)" class="shrink-0">{{ col.data_type }}</span>
          <span
            v-if="col.nullable === false"
            class="shrink-0 text-[10px] font-semibold text-amber-400"
            title="NOT NULL"
          >!</span>
          <!-- Drag port -->
          <div
            class="h-3 w-3 shrink-0 cursor-crosshair rounded-full border-2 border-current transition-colors"
            :class="isSourceLinked(col.name) ? 'bg-emerald-500 border-emerald-500' : 'bg-transparent border-surface-500 group-hover:border-brand-500'"
            @mousedown.prevent.stop="startDrag(col.name, $event)"
          />
        </div>
      </div>

      <!-- Target panel -->
      <div
        class="absolute top-0 flex flex-col"
        :style="{ left: `${tgtX}px`, width: `${srcW}px`, height: `${svgH}px` }"
      >
        <!-- Header -->
        <div class="flex h-10 items-center px-3 text-xs font-semibold text-muted-foreground border-b border-border">
          Target columns
          <span v-if="!targetColumns.length" class="ml-1.5 text-[10px] font-normal opacity-60">(derived)</span>
        </div>
        <!-- Column rows -->
        <div
          v-for="(col, idx) in effectiveTargetCols"
          :key="col.name"
          class="group flex items-center gap-2 px-3 transition-colors"
          :class="isTargetLinked(col.name) ? 'bg-emerald-500/5' : 'hover:bg-muted/60'"
          :style="{ height: `${ROW_H}px` }"
          @mouseup.stop="dropOnTarget(col.name)"
        >
          <!-- Drop port -->
          <div
            class="h-3 w-3 shrink-0 rounded-full border-2 border-current transition-colors"
            :class="[
              isTargetLinked(col.name) ? 'bg-emerald-500 border-emerald-500' : 'bg-transparent border-surface-500 group-hover:border-teal-400',
              drag ? 'cursor-crosshair' : 'cursor-default',
            ]"
          />
          <Key v-if="col.primary_key" class="h-3 w-3 shrink-0 text-brand-500" :stroke-width="2" />
          <span v-else class="h-3 w-3 shrink-0" />
          <span class="min-w-0 flex-1 truncate text-sm font-medium">{{ col.name }}</span>
          <span :class="typeBadge(col.data_type)" class="shrink-0">{{ col.data_type }}</span>
        </div>
      </div>

      <!-- SVG connection layer -->
      <svg
        class="pointer-events-none absolute left-0 top-0"
        :width="containerW"
        :height="svgH"
        style="pointer-events: none"
      >
        <!-- Existing links -->
        <g v-for="link in links" :key="link.id">
          <path
            v-for="(path, pi) in linkPaths(link)"
            :key="pi"
            :d="path"
            fill="none"
            :stroke-width="link.id === selectedLinkId ? 2.5 : 1.75"
            stroke-linecap="round"
            :class="lineColor(link)"
            style="pointer-events: stroke"
            class="cursor-pointer transition-all"
            @click.stop="selectLink(link.id, $event)"
          />
          <!-- Function label on first path -->
          <g
            v-if="lineLabel(link) && linkLabelPos(link)"
            style="pointer-events: all"
            class="cursor-pointer"
            @click.stop="selectLink(link.id, $event)"
          >
            <rect
              :x="linkLabelPos(link)!.x - 28"
              :y="linkLabelPos(link)!.y - 10"
              width="56"
              height="20"
              rx="6"
              class="fill-violet-500/15"
              stroke="rgb(var(--surface-700))"
              stroke-width="1"
            />
            <text
              :x="linkLabelPos(link)!.x"
              :y="linkLabelPos(link)!.y + 4"
              text-anchor="middle"
              class="fill-violet-400 text-[10px]"
              font-size="10"
              font-family="ui-monospace, SFMono-Regular, Menlo, monospace"
            >
              {{ lineLabel(link) }}
            </text>
          </g>
        </g>

        <!-- Drag preview line -->
        <path
          v-if="dragPath"
          :d="dragPath"
          fill="none"
          stroke-width="1.5"
          stroke-dasharray="5 3"
          stroke-linecap="round"
          class="stroke-brand-400/80"
        />
      </svg>
    </div>

    <!-- Function picker panel (shown when a link is selected) -->
    <div
      v-if="selectedLink"
      class="rounded-xl border border-border bg-surface-900 p-4 space-y-4"
    >
      <div class="flex items-start justify-between gap-3">
        <div>
          <p class="text-sm font-semibold flex items-center gap-1.5">
            <Zap class="h-4 w-4 text-brand-500" :stroke-width="1.75" />
            Transform
          </p>
          <p class="mt-0.5 text-xs text-muted-foreground">
            {{ selectedLink.sources.join(" + ") }}
            <span class="mx-1 text-brand-500">→</span>
            {{ selectedLink.target }}
          </p>
        </div>
        <div class="flex items-center gap-2">
          <button
            v-if="selectedLink.fn"
            type="button"
            class="btn-secondary !h-8 !min-h-8 !px-3 !text-xs"
            @click="setFn(null)"
          >
            <RotateCcw class="h-3.5 w-3.5" :stroke-width="1.75" />
            Remove fn
          </button>
          <button type="button" class="btn-secondary !h-8 !min-h-8 !w-8 !px-0" @click="removeLink(selectedLink.id)">
            <X class="h-4 w-4 text-red-400" :stroke-width="1.75" />
          </button>
        </div>
      </div>

      <!-- Source breakdown for multi-source -->
      <div v-if="selectedLink.sources.length > 1" class="flex flex-wrap gap-2">
        <div
          v-for="src in selectedLink.sources"
          :key="src"
          class="flex items-center gap-1.5 rounded-lg border border-border bg-muted/40 px-2 py-1 text-xs"
        >
          {{ src }}
          <button type="button" class="text-muted-foreground hover:text-red-400" @click="removeSrcFromLink(selectedLink.id, src)">
            <X class="h-3 w-3" :stroke-width="2" />
          </button>
        </div>
        <span class="self-center text-xs text-muted-foreground">→ {{ selectedLink.target }}</span>
      </div>

      <!-- Function selector -->
      <div class="space-y-3">
        <div class="flex flex-wrap gap-1.5">
          <button
            v-for="cat in TRANSFORM_CATEGORIES"
            :key="cat"
            type="button"
            class="rounded-lg px-2.5 py-1 text-xs transition-colors"
            :class="pickerFnCategory === cat ? 'bg-brand-500/10 font-semibold text-brand-500' : 'text-muted-foreground hover:text-foreground'"
            @click="pickerFnCategory = cat"
          >
            {{ cat }}
          </button>
        </div>
        <div class="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
          <button
            v-for="fn in TRANSFORM_FUNCTIONS.filter(f => f.category === pickerFnCategory)"
            :key="fn.id"
            type="button"
            class="rounded-xl border p-3 text-left transition-all"
            :class="selectedLink.fn === fn.id
              ? 'border-brand-500 bg-brand-500/10'
              : 'border-border bg-muted/30 hover:border-brand-500/40'"
            @click="setFn(fn.id)"
          >
            <p class="text-sm font-semibold">{{ fn.label }}</p>
            <p class="mt-0.5 text-xs text-muted-foreground line-clamp-2">{{ fn.description }}</p>
            <p v-if="fn.minSources && fn.minSources > 1" class="mt-1 text-[10px] text-brand-400">
              {{ fn.minSources }}–{{ fn.maxSources === 99 ? "N" : fn.maxSources }} source columns
            </p>
          </button>
        </div>
      </div>

      <!-- Function args (shown when a fn is chosen and it has args) -->
      <div v-if="linkFnDef && linkFnDef.args.length" class="grid grid-cols-1 gap-3 rounded-xl border border-border bg-muted/20 p-3 sm:grid-cols-2">
        <p class="sm:col-span-2 text-xs font-semibold text-muted-foreground">{{ linkFnDef.label }} options</p>
        <label v-for="arg in linkFnDef.args" :key="arg.key">
          <span class="auth-label">{{ arg.label }}<span v-if="arg.required" class="ml-1 text-red-400">*</span></span>
          <select
            v-if="arg.type === 'select'"
            :value="(selectedLink.fn_args[arg.key] as string) ?? arg.default ?? ''"
            class="auth-input cursor-pointer"
            @change="setArg(arg.key, ($event.target as HTMLSelectElement).value)"
          >
            <option v-for="opt in arg.options" :key="opt" :value="opt">{{ opt }}</option>
          </select>
          <input
            v-else
            :type="arg.type === 'number' ? 'number' : 'text'"
            :value="(selectedLink.fn_args[arg.key] as string | number) ?? arg.default ?? ''"
            :placeholder="arg.placeholder || String(arg.default ?? '')"
            class="auth-input"
            @input="setArg(arg.key, arg.type === 'number' ? Number(($event.target as HTMLInputElement).value) : ($event.target as HTMLInputElement).value)"
          />
        </label>
        <p class="sm:col-span-2 text-xs text-muted-foreground italic">{{ linkFnDef.example }}</p>
      </div>
    </div>

    <!-- Link summary list (when no picker open) -->
    <div v-else-if="links.length" class="space-y-1.5">
      <p class="text-xs font-medium text-muted-foreground">{{ links.length }} column mapping{{ links.length === 1 ? "" : "s" }}</p>
      <div class="flex flex-col gap-1">
        <div
          v-for="link in links"
          :key="link.id"
          class="flex flex-wrap items-center gap-2 rounded-lg border border-border bg-muted/20 px-3 py-1.5 text-xs"
        >
          <span class="font-medium">{{ link.sources.join(", ") }}</span>
          <span v-if="link.fn" class="badge-neutral !bg-violet-500/10 !text-violet-400">{{ TRANSFORM_BY_ID.get(link.fn)?.label }}</span>
          <span class="text-muted-foreground">→</span>
          <span class="font-medium">{{ link.target }}</span>
          <button
            type="button"
            class="ml-auto flex items-center gap-1 text-muted-foreground hover:text-brand-400"
            @click="selectLink(link.id, $event)"
          >
            <Zap class="h-3 w-3" :stroke-width="1.75" />
            Edit
          </button>
          <button
            type="button"
            class="text-muted-foreground hover:text-red-400"
            @click="removeLink(link.id)"
          >
            <X class="h-3.5 w-3.5" :stroke-width="2" />
          </button>
        </div>
      </div>
    </div>

    <!-- Empty state -->
    <p v-else class="text-xs text-muted-foreground">
      Drag from a source column port to a target column to create a mapping.
      Use <kbd class="rounded border border-border px-1 py-0.5 font-mono text-[10px]">Select all</kbd> to map every column with the same name.
    </p>
  </div>
</template>
