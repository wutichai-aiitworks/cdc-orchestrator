<script setup lang="ts">
/**
 * Chip input for pipeline tags. Enter or comma adds, Backspace on an empty field removes
 * the last chip. Suggestions come from tags already used on other pipelines. Limits match
 * the orchestrator (20 tags, 40 characters, case-insensitive de-duplication).
 */
import { X } from "@lucide/vue"

const MAX_TAGS = 20
const MAX_LEN = 40

const props = defineProps<{
  modelValue: string[]
  suggestions?: string[]
}>()

const emit = defineEmits<{ "update:modelValue": [value: string[]] }>()

const draft = ref("")
const listId = `tag-suggest-${Math.random().toString(36).slice(2, 8)}`

const has = (tag: string) => props.modelValue.some((t) => t.toLowerCase() === tag.toLowerCase())

const available = computed(() => (props.suggestions ?? []).filter((t) => !has(t)))

const hint = computed(() => {
  if (props.modelValue.length >= MAX_TAGS) return `At most ${MAX_TAGS} tags.`
  if (draft.value.trim().length > MAX_LEN) return `A tag is at most ${MAX_LEN} characters.`
  return ""
})

const add = (raw: string) => {
  const parts = raw
    .split(",")
    .map((p) => p.split(/\s+/).filter(Boolean).join(" "))
    .filter(Boolean)
  const next = [...props.modelValue]
  for (const tag of parts) {
    if (tag.length > MAX_LEN || next.length >= MAX_TAGS) continue
    if (next.some((t) => t.toLowerCase() === tag.toLowerCase())) continue
    next.push(tag)
  }
  if (next.length !== props.modelValue.length) emit("update:modelValue", next)
}

const commit = () => {
  if (!draft.value.trim() || draft.value.trim().length > MAX_LEN) return
  add(draft.value)
  draft.value = ""
}

const remove = (tag: string) => emit("update:modelValue", props.modelValue.filter((t) => t !== tag))

const onKeydown = (event: KeyboardEvent) => {
  if (event.key === "Enter" || event.key === ",") {
    event.preventDefault()
    commit()
  } else if (event.key === "Backspace" && !draft.value && props.modelValue.length) {
    remove(props.modelValue[props.modelValue.length - 1]!)
  }
}

// Picking a datalist suggestion fires `input` with the full value: add it right away.
const onInput = (event: Event) => {
  const value = (event.target as HTMLInputElement).value
  draft.value = value
  if (available.value.some((t) => t === value)) commit()
}
</script>

<template>
  <div>
    <div class="auth-input flex min-h-10 flex-wrap items-center gap-1.5 py-1.5">
      <span v-for="tag in modelValue" :key="tag" class="tag-chip">
        {{ tag }}
        <button type="button" class="rounded hover:text-foreground" :aria-label="`Remove tag ${tag}`" @click="remove(tag)">
          <X class="h-3 w-3" :stroke-width="2" />
        </button>
      </span>
      <input
        :value="draft"
        :list="listId"
        class="min-w-[8rem] flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        :placeholder="modelValue.length ? 'Add another…' : 'e.g. crm, master-data, nightly'"
        aria-label="Add tag"
        @input="onInput"
        @keydown="onKeydown"
        @blur="commit"
      />
      <datalist :id="listId">
        <option v-for="tag in available" :key="tag" :value="tag" />
      </datalist>
    </div>
    <p v-if="hint" class="mt-1 text-xs text-amber-600 dark:text-amber-400">{{ hint }}</p>
  </div>
</template>
