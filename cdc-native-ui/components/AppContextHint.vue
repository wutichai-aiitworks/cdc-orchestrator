<script setup lang="ts">
type Tone = "neutral" | "emerald" | "yellow" | "blue" | "brand" | "red"

withDefaults(
  defineProps<{
    label: string
    hint: string
    tone?: Tone
  }>(),
  { tone: "neutral" },
)

const toneClasses: Record<Tone, string> = {
  neutral: "border-border bg-muted/40 text-foreground",
  emerald: "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400",
  yellow: "border-yellow-500/30 bg-yellow-500/10 text-yellow-800 dark:text-yellow-300",
  blue: "border-blue-500/25 bg-blue-500/10 text-blue-700 dark:text-blue-400",
  brand: "border-brand-500/25 bg-brand-500/10 text-brand-800 dark:text-brand-300",
  red: "border-red-500/30 bg-red-500/10 text-red-700 dark:text-red-400",
}
</script>

<template>
  <div
    :class="[
      'flex items-center gap-2 rounded-xl border px-3 py-2 text-sm',
      toneClasses[tone],
    ]"
  >
    <span class="font-medium">{{ label }}</span>
    <AppInfoHint :text="hint" />
  </div>
</template>
