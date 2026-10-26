<script setup lang="ts">
const props = defineProps<{
  modelValue: string | number | null | undefined
  type?: string
  placeholder?: string
  min?: string | number
  list?: string
  inputClass?: string
  disabled?: boolean
  ariaLabel?: string
}>()

const emit = defineEmits<{
  "update:modelValue": [value: string | number]
  enter: []
}>()

const updateValue = (event: Event) => {
  const input = event.target as HTMLInputElement
  emit("update:modelValue", props.type === "number" ? input.valueAsNumber : input.value)
}
</script>

<template>
  <input
    :type="type || 'text'"
    :value="modelValue"
    :min="min"
    :placeholder="placeholder"
    :list="list"
    :disabled="disabled"
    :aria-label="ariaLabel || placeholder"
    :class="[
      'auth-input',
      inputClass || '',
    ]"
    @input="updateValue"
    @change="updateValue"
    @keyup.enter="emit('enter')"
  />
</template>
