<script setup lang="ts">
import type { Component } from "vue"
import { ChevronDown } from "@lucide/vue"
import { onClickOutside, useEventListener } from "@vueuse/core"

export interface ActionMenuItem {
  key: string
  label: string
  icon?: Component
  disabled?: boolean
  destructive?: boolean
  separatorBefore?: boolean
}

const props = withDefaults(
  defineProps<{
    label: string
    items: ActionMenuItem[]
    icon?: Component
    variant?: "primary" | "secondary"
    disabled?: boolean
  }>(),
  { icon: undefined, variant: "secondary", disabled: false },
)

const emit = defineEmits<{ select: [key: string] }>()

const open = ref(false)
const root = ref<HTMLElement | null>(null)
const panel = ref<HTMLElement | null>(null)
const GAP = 6
const EDGE = 8
const pos = ref({ top: 0, left: 0, minWidth: 0, maxHeight: 0, ready: false })

const place = () => {
  if (!open.value || !root.value) return
  const r = root.value.getBoundingClientRect()
  const vv = window.visualViewport
  const viewH = vv?.height ?? window.innerHeight
  const viewW = vv?.width ?? window.innerWidth
  const offsetTop = vv?.offsetTop ?? 0
  const offsetLeft = vv?.offsetLeft ?? 0
  const width = Math.max(panel.value?.offsetWidth ?? 0, r.width, 208)
  const height = panel.value?.scrollHeight ?? 0
  const below = viewH - r.bottom - GAP - EDGE
  const above = r.top - GAP - EDGE
  const up = height > below && above > below
  const maxHeight = Math.max(120, up ? above : below)
  const top = up
    ? Math.max(EDGE, r.top - GAP - Math.min(height, maxHeight)) + offsetTop
    : r.bottom + GAP + offsetTop
  pos.value = {
    top,
    left: Math.max(EDGE, Math.min(r.left, viewW - width - EDGE)) + offsetLeft,
    minWidth: r.width,
    maxHeight,
    ready: true,
  }
}

const schedulePlace = () => {
  pos.value.ready = false
  nextTick(() => {
    place()
    requestAnimationFrame(place)
  })
}

const toggle = () => {
  if (props.disabled) return
  open.value = !open.value
  if (open.value) schedulePlace()
}

const choose = (item: ActionMenuItem) => {
  if (item.disabled) return
  open.value = false
  emit("select", item.key)
}

onClickOutside(
  root,
  () => {
    open.value = false
  },
  { ignore: [panel] },
)

useEventListener(window, "scroll", place, { capture: true, passive: true })
useEventListener(window, "resize", place)
useEventListener(
  () => (import.meta.client ? window.visualViewport : null),
  "resize",
  place,
)
useEventListener(document, "keydown", (event: KeyboardEvent) => {
  if (event.key === "Escape") open.value = false
})

watch(open, (isOpen) => {
  if (isOpen) schedulePlace()
})
</script>

<template>
  <div ref="root" class="relative inline-flex">
    <AppButton
      :variant="variant"
      :disabled="disabled"
      aria-haspopup="menu"
      :aria-expanded="open"
      @click.stop="toggle"
    >
      <component :is="icon" v-if="icon" class="h-4 w-4" />
      {{ label }}
      <ChevronDown
        class="h-4 w-4 opacity-70 transition-transform"
        :class="{ 'rotate-180': open }"
      />
    </AppButton>
    <Teleport to="body">
      <div
        v-if="open"
        ref="panel"
        role="menu"
        class="menu-panel overflow-y-auto overscroll-contain"
        :style="{
          position: 'fixed',
          zIndex: 150,
          top: `${pos.top}px`,
          left: `${pos.left}px`,
          minWidth: pos.minWidth ? `${pos.minWidth}px` : undefined,
          maxHeight: pos.maxHeight ? `${pos.maxHeight}px` : undefined,
          visibility: pos.ready ? 'visible' : 'hidden',
        }"
        @click.stop
      >
        <template v-for="item in items" :key="item.key">
          <div v-if="item.separatorBefore" class="my-1 h-px bg-border" />
          <button
            type="button"
            role="menuitem"
            class="menu-item disabled:pointer-events-none disabled:opacity-50"
            :class="{ '!text-destructive': item.destructive }"
            :disabled="item.disabled"
            @click="choose(item)"
          >
            <component :is="item.icon" v-if="item.icon" class="h-4 w-4" :stroke-width="1.75" />
            {{ item.label }}
          </button>
        </template>
      </div>
    </Teleport>
  </div>
</template>
