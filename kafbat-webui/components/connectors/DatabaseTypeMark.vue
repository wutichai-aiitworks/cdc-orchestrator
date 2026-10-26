<script setup lang="ts">
import { databaseMeta, type ConnectorKind, type ConnectorDatabaseType } from "~/utils/connectorMeta"

const props = withDefaults(
  defineProps<{
    databaseType?: string | null
    kind?: ConnectorKind | string | null
    size?: "sm" | "md" | "lg"
    showKind?: boolean
    subtitle?: string
  }>(),
  {
    databaseType: "unknown",
    kind: null,
    size: "md",
    showKind: true,
    subtitle: "",
  }
)

const meta = computed(() => databaseMeta(props.databaseType as ConnectorDatabaseType))
const kindLabel = computed(() => {
  const kind = String(props.kind || "").toLowerCase()
  if (kind === "source") return "Source"
  if (kind === "sink") return "Sink"
  return ""
})
const tileClass = computed(() => {
  const size = {
    sm: "h-9 w-9 text-sm",
    md: "h-11 w-11 text-sm",
    lg: "h-14 w-14 text-lg",
  }[props.size]
  const tone: Record<string, string> = {
    sky: "bg-sky-500/15 text-sky-700 dark:text-sky-300 ring-sky-500/25",
    cyan: "bg-cyan-500/15 text-cyan-700 dark:text-cyan-300 ring-cyan-500/25",
    amber: "bg-amber-500/15 text-amber-800 dark:text-amber-300 ring-amber-500/25",
    red: "bg-red-500/15 text-red-700 dark:text-red-300 ring-red-500/25",
    orange: "bg-brand-500/15 text-primary ring-brand-500/30",
    slate: "bg-muted text-muted-foreground ring-border",
  }
  return `${size} ${tone[meta.value.tone] || tone.slate}`
})
const kindClass = computed(() =>
  String(props.kind).toLowerCase() === "sink"
    ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-400"
    : "bg-blue-500/15 text-blue-700 dark:text-blue-400"
)
</script>

<template>
  <div class="flex min-w-0 items-center gap-3">
    <div
      class="flex flex-shrink-0 items-center justify-center rounded-xl font-display font-bold ring-1"
      :class="tileClass"
      :title="meta.label"
    >
      {{ meta.monogram }}
    </div>
    <div class="min-w-0">
      <div class="flex flex-wrap items-center gap-1.5">
        <p class="truncate font-medium text-foreground" :class="size === 'lg' ? 'text-lg' : 'text-sm'">
          {{ meta.label }}
        </p>
        <span v-if="showKind && kindLabel" class="rounded-full px-2 py-0.5 text-sm font-medium" :class="kindClass">
          {{ kindLabel }}
        </span>
      </div>
      <p v-if="subtitle" class="mt-0.5 truncate text-sm text-muted-foreground">{{ subtitle }}</p>
    </div>
  </div>
</template>
