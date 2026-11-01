<script setup lang="ts">
const props = defineProps<{
  page: number
  totalPages: number
  disabled?: boolean
}>()

const emit = defineEmits<{
  prev: []
  next: []
}>()
</script>

<template>
  <div class="flex items-center gap-2">
    <AppButton :disabled="disabled || page <= 1" @click="emit('prev')">Previous</AppButton>
    <span class="px-1 text-sm text-muted-foreground">Page {{ page }} of {{ totalPages }}</span>
    <AppButton :disabled="disabled || page >= totalPages" @click="emit('next')">Next</AppButton>
  </div>
</template>
