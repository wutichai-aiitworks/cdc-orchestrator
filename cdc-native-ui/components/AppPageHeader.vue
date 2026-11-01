<script setup lang="ts">
defineProps<{
  title: string
  description?: string
}>()
</script>

<template>
  <section class="page-toolbar">
    <div class="min-w-0">
      <div class="flex items-center gap-2">
        <h1 class="page-title">{{ title }}</h1>
        <AppInfoHint v-if="description" :text="description" />
      </div>
    </div>
    <div class="flex flex-wrap items-center gap-2">
      <slot />
    </div>
  </section>
</template>
