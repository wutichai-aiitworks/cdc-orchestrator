<script setup lang="ts" generic="T extends string">
// Small exclusive choice (2–4 options), keyboard operable as a radio group.
const model = defineModel<T>({ required: true })
const props = defineProps<{ options: { value: T; label: string; help?: string }[]; label: string; disabled?: boolean }>()

const move = (dir: number) => {
  const i = props.options.findIndex((o) => o.value === model.value)
  const next = props.options[(i + dir + props.options.length) % props.options.length]
  if (next) model.value = next.value
}
const current = computed(() => props.options.find((o) => o.value === model.value))
</script>

<template>
  <div>
    <div
      role="radiogroup"
      :aria-label="label"
      class="inline-flex max-w-full flex-wrap rounded-lg border border-border bg-muted p-0.5"
      :class="disabled ? 'pointer-events-none opacity-50' : ''"
      @keydown.right.prevent="move(1)"
      @keydown.left.prevent="move(-1)"
    >
      <button
        v-for="o in options"
        :key="o.value"
        type="button"
        role="radio"
        :aria-checked="model === o.value"
        :tabindex="model === o.value ? 0 : -1"
        class="whitespace-nowrap rounded-md px-3 py-1.5 text-sm transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        :class="model === o.value ? 'bg-card font-medium text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'"
        @click="model = o.value"
      >
        {{ o.label }}
      </button>
    </div>
    <p v-if="current?.help" class="mt-1.5 text-xs text-muted-foreground">{{ current.help }}</p>
  </div>
</template>
