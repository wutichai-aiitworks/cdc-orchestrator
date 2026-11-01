<script setup lang="ts">
defineProps<{
  label: string
  hint?: string
}>()
</script>

<template>
  <div class="mb-2 flex items-center gap-1.5">
    <span class="auth-label mb-0">{{ label }}</span>
    <AppInfoHint v-if="hint || $slots.default" :text="hint">
      <slot />
    </AppInfoHint>
  </div>
</template>
