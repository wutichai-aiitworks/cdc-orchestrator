<script setup lang="ts">
// One configuration section: title + why on the left, fields on the right. Sections are divided
// by a rule and space, never nested cards.
defineProps<{ id: string; title: string; why?: string }>()
</script>

<template>
  <section :id="id" class="grid scroll-mt-4 gap-4 border-t border-border py-6 first:border-t-0 first:pt-0 lg:grid-cols-[15rem_minmax(0,1fr)] lg:gap-8">
    <div>
      <h3 class="text-sm font-semibold text-foreground">{{ title }}</h3>
      <p v-if="why" class="mt-1 text-xs leading-5 text-muted-foreground">{{ why }}</p>
      <slot name="aside" />
    </div>
    <div class="min-w-0 space-y-4">
      <slot />
    </div>
  </section>
</template>
