<script setup lang="ts">
withDefaults(
  defineProps<{
    title?: string
    hint?: string
    tag?: "h2" | "h3" | "p"
  }>(),
  { title: "", hint: "", tag: "h2" },
)
</script>

<template>
  <div class="flex min-w-0 items-center gap-1.5">
    <component
      :is="tag"
      :class="
        tag === 'h2'
          ? 'section-title'
          : tag === 'h3'
            ? 'text-sm font-display font-semibold text-foreground'
            : 'text-sm font-medium text-foreground'
      "
    >
      <slot>{{ title }}</slot>
    </component>
    <AppInfoHint v-if="hint || $slots.hint" :text="hint">
      <slot name="hint" />
    </AppInfoHint>
  </div>
</template>
