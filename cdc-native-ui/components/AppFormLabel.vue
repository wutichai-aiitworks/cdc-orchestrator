<script setup lang="ts">
defineProps<{
  label: string
  hint?: string
}>()
</script>

<template>
  <div class="auth-label">
    <span>{{ label }}</span>
    <AppInfoHint v-if="hint || $slots.default" :text="hint">
      <slot />
    </AppInfoHint>
  </div>
</template>
