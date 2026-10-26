<script setup lang="ts">
defineProps<{
  brokerId: string | number
  active: "logdirs" | "configs" | "metrics"
}>()

const tabs = [
  { id: "logdirs", label: "Log directories" },
  { id: "configs", label: "Configs" },
  { id: "metrics", label: "Metrics" },
] as const
</script>

<template>
  <nav class="flex flex-wrap gap-1 border-b border-border pb-px" role="navigation">
    <NuxtLink
      v-for="tab in tabs"
      :key="tab.id"
      :to="{ path: `/brokers/${brokerId}`, query: { tab: tab.id } }"
      :class="[
        'rounded-t-lg px-3 py-2 text-sm font-medium transition-colors',
        active === tab.id
          ? 'bg-brand-500/15 text-primary'
          : 'text-muted-foreground hover:bg-accent hover:text-foreground',
      ]"
    >
      {{ tab.label }}
    </NuxtLink>
  </nav>
</template>
