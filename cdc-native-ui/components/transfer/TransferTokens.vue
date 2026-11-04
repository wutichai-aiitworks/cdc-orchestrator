<script setup lang="ts">
// List input for names and patterns: Enter, comma or paste adds; Backspace on an empty field
// removes the last one. Values are shown in monospace because people copy them.
import { X } from "@lucide/vue"

const model = defineModel<string[]>({ default: () => [] })
const props = defineProps<{ label: string; placeholder?: string; help?: string; disabled?: boolean }>()
const draft = ref("")
const id = useId()

const add = (raw: string) => {
  const parts = raw.split(/[\n,]/).map((s) => s.trim()).filter(Boolean)
  if (!parts.length) return
  model.value = [...model.value, ...parts.filter((p) => !model.value.includes(p))]
  draft.value = ""
}
const remove = (i: number) => (model.value = model.value.filter((_, j) => j !== i))
const onKey = (e: KeyboardEvent) => {
  if (e.key === "Enter" || e.key === ",") {
    e.preventDefault()
    add(draft.value)
  } else if (e.key === "Backspace" && !draft.value && model.value.length) {
    remove(model.value.length - 1)
  }
}
const onPaste = (e: ClipboardEvent) => {
  const text = e.clipboardData?.getData("text") || ""
  if (/[\n,]/.test(text)) {
    e.preventDefault()
    add(text)
  }
}
</script>

<template>
  <div>
    <label :for="id" class="text-sm font-medium text-foreground">{{ props.label }}</label>
    <p v-if="help" class="text-xs text-muted-foreground">{{ help }}</p>
    <div
      class="mt-1.5 flex min-h-10 flex-wrap items-center gap-1.5 rounded-xl border border-input bg-muted/60 px-2 py-1.5 focus-within:ring-1 focus-within:ring-ring"
      :class="disabled ? 'pointer-events-none opacity-50' : ''"
    >
      <span
        v-for="(v, i) in model"
        :key="v"
        class="inline-flex max-w-full items-center gap-1 rounded-md border border-border bg-muted px-2 py-0.5 font-mono text-xs text-foreground"
      >
        <span class="truncate">{{ v }}</span>
        <button type="button" class="text-muted-foreground hover:text-foreground" :aria-label="`Remove ${v}`" @click="remove(i)">
          <X class="h-3 w-3" :stroke-width="2" />
        </button>
      </span>
      <input
        :id="id"
        v-model="draft"
        class="min-w-[8rem] flex-1 bg-transparent px-1 font-mono text-xs text-foreground outline-none placeholder:font-sans placeholder:text-muted-foreground"
        :placeholder="model.length ? '' : placeholder"
        :disabled="disabled"
        @keydown="onKey"
        @paste="onPaste"
        @blur="add(draft)"
      />
    </div>
  </div>
</template>
