<script setup lang="ts">
// KPI tile: small label, the number as the headline. The icon stays muted unless its tone
// carries meaning (warning / danger), so colour points at problems only.
withDefaults(
  defineProps<{
    label: string
    value: string | number
    helper?: string
    tone?: "brand" | "success" | "warning" | "danger" | "error" | "info" | "neutral"
  }>(),
  { helper: "", tone: "neutral" }
)

const iconClass = {
  brand: "text-muted-foreground",
  success: "text-muted-foreground",
  info: "text-muted-foreground",
  neutral: "text-muted-foreground",
  warning: "text-amber-600 dark:text-amber-400",
  danger: "text-red-600 dark:text-red-400",
  error: "text-red-600 dark:text-red-400",
}
const valueClass = {
  brand: "text-foreground",
  success: "text-foreground",
  info: "text-foreground",
  neutral: "text-foreground",
  warning: "text-amber-700 dark:text-amber-400",
  danger: "text-red-700 dark:text-red-400",
  error: "text-red-700 dark:text-red-400",
}
/** Counts and ratios ("17 / 17", "4:48 PM") read as headlines; names and IDs need a smaller size. */
const isFigure = (v: string | number) => typeof v === "number" || /^[\d\s.,:/%—–+-]+(\s?[A-Za-z]{1,3})?$/.test(String(v))
</script>

<template>
  <section class="stat-card">
    <div class="flex items-center gap-1.5 text-muted-foreground">
      <span v-if="$slots.default" :class="['flex h-3.5 w-3.5 shrink-0 items-center justify-center [&>svg]:h-3.5 [&>svg]:w-3.5', iconClass[tone]]" aria-hidden="true">
        <slot />
      </span>
      <p class="truncate text-xs font-medium">{{ label }}</p>
      <AppInfoHint v-if="helper" :text="helper" />
    </div>
    <p
      class="mt-1.5 truncate font-semibold tracking-tight"
      :class="[valueClass[tone], isFigure(value) ? 'text-2xl tabular-nums' : 'text-base leading-8']"
      :title="String(value)"
    >{{ value }}</p>
  </section>
</template>
