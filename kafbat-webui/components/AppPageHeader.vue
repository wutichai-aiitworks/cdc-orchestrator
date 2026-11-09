<script setup lang="ts">
// Page title with its one-line purpose shown (not hidden behind an info icon); actions on the right.
defineProps<{
  title: string
  description?: string
}>()
</script>

<template>
  <section class="page-toolbar">
    <div class="min-w-0">
      <h1 class="page-title">{{ title }}</h1>
      <p v-if="description" class="mt-0.5 max-w-3xl text-sm text-muted-foreground">{{ description }}</p>
    </div>
    <div class="flex flex-wrap items-center gap-2">
      <slot />
    </div>
  </section>
</template>
