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
const pos = ref({ top: 0, left: 0, width: 0 })
const listId = `combo-${Math.random().toString(36).slice(2, 9)}`

const current = computed(() => String(props.modelValue ?? ""))

const visible = computed(() => {
  const unique = [...new Set(props.options.map((o) => o.trim()).filter(Boolean))]
  if (!typed.value) return unique
  const q = current.value.trim().toLowerCase()
  return q ? unique.filter((o) => o.toLowerCase().includes(q)) : unique
})

const place = () => {
  if (!open.value || !root.value) return
  const r = root.value.getBoundingClientRect()
  pos.value = { top: r.bottom + 4, left: r.left, width: r.width }
}

const show = () => {
  if (props.disabled) return
  open.value = true
  typed.value = false
  active.value = visible.value.findIndex((o) => o.toLowerCase() === current.value.trim().toLowerCase())
  nextTick(place)
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
    nextTick(place)
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
        class="menu-panel max-h-64 overflow-auto"
        :style="{ top: `${pos.top}px`, left: `${pos.left}px`, minWidth: `${pos.width}px` }"
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
