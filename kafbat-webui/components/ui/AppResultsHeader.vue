<script setup lang="ts">
defineProps<{
  label: string
  total: number
  page?: number
  totalPages?: number
  loading?: boolean
  lastUpdated?: string
}>()
</script>

<template>
  <div class="flex items-center justify-between text-sm text-muted-foreground gap-3 flex-wrap">
    <p>
      {{ label }}: {{ total }}
      <span v-if="page && totalPages"> · Page {{ page }} / {{ totalPages }}</span>
      <span v-if="loading"> · refreshing…</span>
    </p>
    <p v-if="lastUpdated">Updated {{ lastUpdated }}</p>
  </div>
</template>
