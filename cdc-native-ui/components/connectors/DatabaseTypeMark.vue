<script setup lang="ts">
import { databaseMeta, type ConnectorKind, type ConnectorDatabaseType } from "~/utils/connectorMeta"
import { dbIconPath } from "~/utils/dbIcon"

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
// Vendor logo (same images as the pipeline pages); unknown vendors get the generic database icon.
const iconSrc = computed(() => dbIconPath(meta.value.value === "unknown" ? null : meta.value.value))
const tileClass = computed(
  () =>
    ({
      sm: "h-9 w-9 p-1.5",
      md: "h-11 w-11 p-2",
      lg: "h-14 w-14 p-2.5",
    })[props.size],
)
const kindClass = computed(() =>
  String(props.kind).toLowerCase() === "sink"
    ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-400"
    : "bg-blue-500/15 text-blue-700 dark:text-blue-400"
)
</script>

<template>
  <div class="flex min-w-0 items-center gap-3">
    <div
      class="flex flex-shrink-0 items-center justify-center rounded-xl bg-card ring-1 ring-border"
      :class="tileClass"
      :title="meta.label"
    >
      <img :src="iconSrc" :alt="meta.label" class="h-full w-full object-contain" />
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
