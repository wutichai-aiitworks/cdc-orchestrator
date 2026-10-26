<script setup lang="ts">
defineProps<{
  topic: string
  active: "overview" | "messages" | "consumers" | "settings" | "statistics" | "connectors" | "acls"
}>()

const tabs = [
  { id: "overview", label: "Overview" },
  { id: "messages", label: "Messages" },
  { id: "consumers", label: "Consumers" },
  { id: "settings", label: "Settings" },
  { id: "statistics", label: "Statistics" },
  { id: "connectors", label: "Connectors" },
  { id: "acls", label: "ACL" },
] as const
</script>

<template>
  <nav class="flex flex-wrap gap-1 border-b border-border pb-px">
    <NuxtLink
      v-for="tab in tabs"
      :key="tab.id"
      :to="{ path: `/topics/${encodeURIComponent(topic)}`, query: { tab: tab.id } }"
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
