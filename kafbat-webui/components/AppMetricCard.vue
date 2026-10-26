<script setup lang="ts">
withDefaults(
  defineProps<{
    label: string
    value: string | number
    helper?: string
    tone?: "brand" | "success" | "warning" | "danger" | "info" | "neutral"
  }>(),
  { helper: "", tone: "neutral" }
)

const toneClass = {
  brand: "bg-brand-500/12 text-primary",
  success: "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400",
  warning: "bg-amber-500/12 text-amber-700 dark:text-amber-400",
  danger: "bg-red-500/12 text-red-700 dark:text-red-400",
  info: "bg-blue-500/12 text-blue-700 dark:text-blue-400",
  neutral: "bg-muted text-muted-foreground",
}
</script>

<template>
  <section class="stat-card">
    <div class="flex items-start justify-between gap-2">
      <div class="min-w-0">
        <div class="flex items-center gap-1">
          <p class="text-xs font-medium text-muted-foreground">{{ label }}</p>
          <AppInfoHint v-if="helper" :text="helper" />
        </div>
        <p class="mt-1 text-sm font-semibold text-foreground">{{ value }}</p>
      </div>
      <div v-if="$slots.default" :class="['flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-lg', toneClass[tone]]">
        <slot />
      </div>
    </div>
  </section>
</template>
