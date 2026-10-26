<script setup lang="ts">
defineProps<{
  modelValue: string | number
  selectClass?: string
  disabled?: boolean
  ariaLabel?: string
}>()

const emit = defineEmits<{
  "update:modelValue": [value: string]
}>()
</script>

<template>
  <select
    :value="modelValue"
    :disabled="disabled"
    :aria-label="ariaLabel"
    :class="[
      'auth-input cursor-pointer',
      selectClass || '',
    ]"
    @change="emit('update:modelValue', ($event.target as HTMLSelectElement).value)"
  >
    <slot />
  </select>
</template>
