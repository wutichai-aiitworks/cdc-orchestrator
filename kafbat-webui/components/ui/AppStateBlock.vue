<script setup lang="ts">
withDefaults(
  defineProps<{
    kind: "loading" | "error" | "empty" | "success"
    message: string
  }>(),
  {
    kind: "empty",
  }
)
</script>

<template>
  <div
    class="text-sm"
    :class="
      kind === 'error'
        ? 'alert-error'
        : kind === 'success'
          ? 'alert-success'
        : kind === 'loading'
          ? 'alert-info'
          : 'rounded-xl border border-border bg-muted px-4 py-3 text-muted-foreground'
    "
  >
    {{ message }}
  </div>
</template>
