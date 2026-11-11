<script setup lang="ts">
/**
 * Free-text input with a suggestion list. Replaces <datalist>, which browsers filter
 * by the text already in the box — a pre-filled value (editing a saved pipeline)
 * leaves the dropdown empty. Here, opening the list shows every option; it filters
 * only once the user types.
 */
import { Check, ChevronDown } from "@lucide/vue"
import { onClickOutside, useEventListener } from "@vueuse/core"

const props = defineProps<{
  modelValue: string | null | undefined
  options: string[]
  placeholder?: string
  ariaLabel?: string
  disabled?: boolean
  emptyText?: string
}>()

const emit = defineEmits<{
  "update:modelValue": [value: string]
}>()

const root = ref<HTMLElement | null>(null)
const input = ref<HTMLInputElement | null>(null)
const panel = ref<HTMLElement | null>(null)
const open = ref(false)
const typed = ref(false)
const active = ref(-1)
const GAP = 4
const EDGE = 8
const MAX_PANEL = 256 // keeps the old max-h-64 cap when the viewport has room
const pos = ref({ top: 0, left: 0, width: 0, maxHeight: MAX_PANEL, ready: false })
const listId = `combo-${Math.random().toString(36).slice(2, 9)}`

const current = computed(() => String(props.modelValue ?? ""))

const visible = computed(() => {
  const unique = [...new Set(props.options.map((o) => o.trim()).filter(Boolean))]
  if (!typed.value) return unique
  const q = current.value.trim().toLowerCase()
  return q ? unique.filter((o) => o.toLowerCase().includes(q)) : unique
})

/**
 * Viewport-aware placement (same approach as AppActionMenu). A row near the bottom
 * of the page used to push the list off-screen; now the list flips above the input
 * when there is more room there, and its height is capped to the free space so it
 * scrolls instead of being clipped.
 */
const place = () => {
  if (!open.value || !root.value) return
  const r = root.value.getBoundingClientRect()
  const vv = window.visualViewport
  const viewH = vv?.height ?? window.innerHeight
  const viewW = vv?.width ?? window.innerWidth
  const offsetTop = vv?.offsetTop ?? 0
  const offsetLeft = vv?.offsetLeft ?? 0
  const width = Math.max(panel.value?.offsetWidth ?? 0, r.width)
  const height = Math.min(panel.value?.scrollHeight ?? 0, MAX_PANEL)
  const below = viewH - r.bottom - GAP - EDGE
  const above = r.top - GAP - EDGE
  const up = height > below && above > below
  const maxHeight = Math.min(MAX_PANEL, Math.max(120, up ? above : below))
  const top = up
    ? Math.max(EDGE, r.top - GAP - Math.min(height, maxHeight)) + offsetTop
    : r.bottom + GAP + offsetTop
  pos.value = {
    top,
    left: Math.max(EDGE, Math.min(r.left, viewW - width - EDGE)) + offsetLeft,
    width: r.width,
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

const show = () => {
  if (props.disabled) return
  open.value = true
  typed.value = false
  active.value = visible.value.findIndex((o) => o.toLowerCase() === current.value.trim().toLowerCase())
  schedulePlace()
}

const hide = () => {
  open.value = false
  active.value = -1
}

const pick = (value: string) => {
  emit("update:modelValue", value)
  hide()
  input.value?.focus()
}

const onInput = (event: Event) => {
  emit("update:modelValue", (event.target as HTMLInputElement).value)
  typed.value = true
  active.value = -1
  if (!open.value) {
    open.value = true
    schedulePlace()
  }
}

const onKeydown = (event: KeyboardEvent) => {
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault()
    if (!open.value) return show()
    const n = visible.value.length
    if (!n) return
    active.value = event.key === "ArrowDown" ? (active.value + 1) % n : (active.value - 1 + n) % n
    nextTick(() => panel.value?.querySelector<HTMLElement>(`[data-index="${active.value}"]`)?.scrollIntoView({ block: "nearest" }))
  } else if (event.key === "Enter" && open.value && active.value >= 0) {
    event.preventDefault()
    pick(visible.value[active.value])
  } else if (event.key === "Escape" && open.value) {
    event.stopPropagation()
    hide()
  } else if (event.key === "Tab") {
    hide()
  }
}

// Typing filters the list and changes its height; when the list sits above the
// input it must be re-anchored so its bottom edge stays next to the field.
watch(visible, () => {
  if (open.value) nextTick(place)
})

onClickOutside(root, hide, { ignore: [panel] })
useEventListener(window, "scroll", place, { capture: true, passive: true })
useEventListener(window, "resize", place)
</script>

<template>
  <div ref="root" class="relative">
    <input
      ref="input"
      type="text"
      role="combobox"
      autocomplete="off"
      :value="current"
      :placeholder="placeholder"
      :disabled="disabled"
      :aria-label="ariaLabel || placeholder"
      :aria-expanded="open"
      :aria-controls="listId"
      aria-autocomplete="list"
      class="auth-input pr-9"
      @input="onInput"
      @focus="show"
      @click="!open && show()"
      @keydown="onKeydown"
    />
    <button
      type="button"
      tabindex="-1"
      class="absolute inset-y-0 right-0 flex w-9 items-center justify-center text-muted-foreground hover:text-foreground"
      :disabled="disabled"
      aria-label="Show options"
      @mousedown.prevent
      @click="open ? hide() : (show(), input?.focus())"
    >
      <ChevronDown class="h-4 w-4 transition-transform" :class="{ 'rotate-180': open }" :stroke-width="1.75" />
    </button>
    <Teleport to="body">
      <div
        v-if="open"
        :id="listId"
        ref="panel"
        role="listbox"
        class="menu-panel overflow-auto overscroll-contain"
        :style="{
          top: `${pos.top}px`,
          left: `${pos.left}px`,
          minWidth: `${pos.width}px`,
          maxHeight: `${pos.maxHeight}px`,
          visibility: pos.ready ? 'visible' : 'hidden',
        }"
      >
        <button
          v-for="(option, i) in visible"
          :key="option"
          type="button"
          role="option"
          :data-index="i"
          :aria-selected="option === current"
          class="menu-item font-mono"
          :class="{ 'bg-accent': i === active }"
          @mousedown.prevent
          @click="pick(option)"
        >
          <Check
            class="h-4 w-4 shrink-0 text-primary"
            :class="option.toLowerCase() === current.trim().toLowerCase() ? 'opacity-100' : 'opacity-0'"
            :stroke-width="1.75"
          />
          <span class="truncate">{{ option }}</span>
        </button>
        <p v-if="!visible.length" class="px-3 py-2 text-sm text-muted-foreground">
          {{ typed && current ? `No match — "${current}" will be used as typed` : emptyText || "No options yet" }}
        </p>
      </div>
    </Teleport>
  </div>
</template>
