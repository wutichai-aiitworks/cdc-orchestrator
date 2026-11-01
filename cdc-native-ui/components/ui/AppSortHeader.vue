<script setup lang="ts">
import { ChevronDown, ChevronUp, ChevronsUpDown } from "@lucide/vue"

const props = defineProps<{
  label: string
  column: string
  sortBy: string
  sortOrder: "asc" | "desc"
}>()

const emit = defineEmits<{
  sort: [column: string]
}>()

const active = computed(() => props.sortBy === props.column)
</script>

<template>
  <button
    type="button"
    class="inline-flex items-center gap-1 text-left text-sm font-medium text-muted-foreground hover:text-foreground"
    @click="emit('sort', column)"
  >
    {{ label }}
    <ChevronUp v-if="active && sortOrder === 'asc'" class="h-3.5 w-3.5" :stroke-width="1.75" />
    <ChevronDown v-else-if="active && sortOrder === 'desc'" class="h-3.5 w-3.5" :stroke-width="1.75" />
    <ChevronsUpDown v-else class="h-3.5 w-3.5 opacity-50" :stroke-width="1.75" />
  </button>
</template>
