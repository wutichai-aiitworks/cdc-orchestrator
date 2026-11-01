<script setup lang="ts">
defineProps<{
  title: string
  description: string
}>()
</script>

<template>
  <div class="flex flex-col items-center justify-center rounded-xl border border-dashed border-border bg-muted/40 px-6 py-10 text-center">
    <div class="mb-3 flex h-11 w-11 items-center justify-center rounded-xl bg-brand-500/10 text-primary">
      <slot name="icon" />
    </div>
    <h3 class="section-title">{{ title }}</h3>
    <p class="mt-1 max-w-md text-sm text-muted-foreground">{{ description }}</p>
    <div class="mt-4">
      <slot />
    </div>
  </div>
</template>
