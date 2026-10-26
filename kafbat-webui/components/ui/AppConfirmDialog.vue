<script setup lang="ts">
const props = defineProps<{
  open: boolean
  title: string
  confirmLabel?: string
  danger?: boolean
  loading?: boolean
}>()

const emit = defineEmits<{
  "update:open": [value: boolean]
  confirm: []
}>()

const close = () => {
  if (!props.loading) emit("update:open", false)
}
</script>

<template>
  <div v-if="open" class="modal-overlay" @click.self="close">
    <div class="w-full max-w-sm rounded-xl border border-border bg-card p-5 shadow-2xl">
      <h3 class="text-base font-semibold text-foreground">{{ title }}</h3>
      <div class="mt-3 text-sm text-muted-foreground">
        <slot />
      </div>
      <div class="mt-5 flex justify-end gap-2">
        <AppButton :disabled="loading" @click="close">Cancel</AppButton>
        <AppButton
          :variant="danger ? 'destructive' : 'primary'"
          :disabled="loading"
          @click="emit('confirm')"
        >
          {{ loading ? "Working…" : confirmLabel || "Confirm" }}
        </AppButton>
      </div>
    </div>
  </div>
</template>
