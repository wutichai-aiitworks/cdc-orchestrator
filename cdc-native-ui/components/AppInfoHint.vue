<script setup lang="ts">
import { Info } from "@lucide/vue"
import { onClickOutside, useEventListener } from "@vueuse/core"

const props = withDefaults(
  defineProps<{
    text?: string
    ariaLabel?: string
  }>(),
  { text: "", ariaLabel: "" },
)

const open = ref(false)
const pinned = ref(false)
const root = ref<HTMLElement | null>(null)
const tip = ref<HTMLElement | null>(null)
const pos = ref({ top: 0, left: 0, width: 320 })

const accessibleLabel = computed(
  () => props.ariaLabel || props.text || "More information",
)

const hasContent = computed(() => Boolean(props.text || useSlots().default))

const place = () => {
  if (!open.value || !root.value) return
  const r = root.value.getBoundingClientRect()
  const pad = 8
  const gap = 8
  const width = Math.min(320, window.innerWidth - pad * 2)
  const height = tip.value?.offsetHeight || 0
  const rightLeft = r.right + gap
  const belowTop = r.bottom + gap
  const aboveTop = r.top - gap - height
  let left = r.left
  let top = belowTop
  if (rightLeft + width <= window.innerWidth - pad) {
    left = rightLeft
    top = r.top
  } else if (belowTop + height > window.innerHeight - pad && aboveTop >= pad) {
    top = aboveTop
  }
  if (left + width > window.innerWidth - pad) left = window.innerWidth - pad - width
  if (left < pad) left = pad
  if (top + height > window.innerHeight - pad) top = Math.max(pad, window.innerHeight - pad - height)
  if (top < pad) top = pad
  pos.value = { top, left, width }
}

const show = () => {
  if (!hasContent.value) return
  open.value = true
  nextTick(place)
}

const hide = () => {
  if (pinned.value) return
  open.value = false
}

const toggle = () => {
  if (!hasContent.value) return
  pinned.value = !pinned.value
  open.value = pinned.value
  if (open.value) nextTick(place)
}

watch(open, (value) => {
  if (value) nextTick(place)
})

onClickOutside(
  root,
  () => {
    pinned.value = false
    open.value = false
  },
  { ignore: [tip] },
)

useEventListener(window, "scroll", place, { capture: true, passive: true })
useEventListener(window, "resize", place)
useEventListener(document, "keydown", (event: KeyboardEvent) => {
  if (event.key !== "Escape") return
  pinned.value = false
  open.value = false
})
</script>

<template>
  <span
    ref="root"
    class="relative inline-flex shrink-0 align-middle"
    @mouseenter="show"
    @mouseleave="hide"
  >
    <button
      type="button"
      class="inline-flex h-4 w-4 items-center justify-center rounded-full text-muted-foreground transition-colors hover:text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
      :aria-label="accessibleLabel"
      :aria-expanded="open"
      @click.stop="toggle"
    >
      <Info class="h-3.5 w-3.5" :stroke-width="1.75" />
    </button>
    <Teleport to="body">
      <div
        v-if="open && hasContent"
        ref="tip"
        class="pointer-events-auto fixed z-[80] rounded-xl border border-border bg-popover p-3 text-xs leading-relaxed text-popover-foreground shadow-lg"
        role="tooltip"
        :style="{ top: `${pos.top}px`, left: `${pos.left}px`, width: `${pos.width}px` }"
        @mouseenter="show"
        @mouseleave="hide"
      >
        <slot>{{ text }}</slot>
      </div>
    </Teleport>
  </span>
</template>
