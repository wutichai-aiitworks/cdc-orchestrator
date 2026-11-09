<script setup lang="ts">
import {
  Activity,
  AlertCircle,
  AlertTriangle,
  Ban,
  CheckCircle2,
  FileText,
  HelpCircle,
  Info,
  Loader2,
  Pause,
  Square,
} from "@lucide/vue"
import type { Component } from "vue"

const props = withDefaults(
  defineProps<{
    status?: string | null
    label?: string
  }>(),
  { status: "unknown", label: "" }
)

type Motion = "spin" | "pulse" | "none"

const normalized = computed(() => String(props.status || "unknown").toLowerCase().trim())

const tokens = computed(() => {
  const raw = normalized.value.replace(/[^a-z0-9]+/g, " ").trim()
  return new Set(raw.split(/\s+/).filter(Boolean))
})

const has = (...keys: string[]) => keys.some((key) => tokens.value.has(key))

const visual = computed((): { tone: string; icon: Component; motion: Motion } => {
  if (has("checking", "starting", "restarting", "rebalancing", "preparing", "pending", "testing", "loading")) {
    return { tone: "badge-warning", icon: Loader2, motion: "spin" }
  }
  if (has("running", "healthy", "online", "live", "stable", "up")) {
    return { tone: "badge-success", icon: Activity, motion: "none" }
  }
  if (has("ok", "success", "enabled", "ready", "resolved", "recovered", "valid", "connected", "configured", "allow")) {
    return { tone: "badge-success", icon: CheckCircle2, motion: "none" }
  }
  if (has("failed", "error", "unhealthy", "dead", "down", "offline", "invalid", "unassigned")) {
    return { tone: "badge-error", icon: AlertCircle, motion: "pulse" }
  }
  if (has("paused", "pause")) {
    return { tone: "badge-warning", icon: Pause, motion: "none" }
  }
  if (has("stopped", "exited", "stop")) {
    return { tone: "badge-warning", icon: Square, motion: "none" }
  }
  if (has("warning", "warn", "degraded", "open")) {
    return { tone: "badge-warning", icon: AlertTriangle, motion: "none" }
  }
  if (has("draft")) {
    return { tone: "badge-info", icon: FileText, motion: "none" }
  }
  if (has("disabled", "empty")) {
    return { tone: "badge-neutral", icon: Ban, motion: "none" }
  }
  if (has("info")) {
    return { tone: "badge-info", icon: Info, motion: "none" }
  }
  if (has("debug", "trace")) {
    return { tone: "badge-neutral", icon: Info, motion: "none" }
  }
  return { tone: "badge-neutral", icon: HelpCircle, motion: "none" }
})

/** "RUNNING" / "UNASSIGNED_TASK" → "Running" / "Unassigned task"; mixed-case labels stay as given. */
const display = computed(() => {
  const raw = String(props.label || props.status || "Unknown")
  if (raw !== raw.toUpperCase() || !/[A-Z]/.test(raw)) return raw
  const words = raw.toLowerCase().replace(/_/g, " ")
  return words.charAt(0).toUpperCase() + words.slice(1)
})
/** Errors and in-progress keep an icon (they need attention / are moving); the rest is a dot. */
const showIcon = computed(() => visual.value.motion !== "none" || visual.value.tone === "badge-error")
const DOT: Record<string, string> = {
  "badge-success": "bg-emerald-500",
  "badge-warning": "bg-amber-500",
  "badge-info": "bg-blue-500",
  "badge-neutral": "bg-zinc-400 dark:bg-zinc-500",
  "badge-error": "bg-red-500",
}
</script>

<template>
  <span :class="visual.tone" role="status" :title="display">
    <span v-if="!showIcon" class="h-1.5 w-1.5 shrink-0 rounded-full" :class="DOT[visual.tone]" aria-hidden="true" />
    <span v-else class="relative inline-flex h-3 w-3 shrink-0 items-center justify-center">
      <component
        :is="visual.icon"
        class="relative h-3 w-3"
        :class="{
          'animate-spin motion-reduce:animate-none': visual.motion === 'spin',
          'animate-pulse motion-reduce:animate-none': visual.motion === 'pulse',
        }"
        :stroke-width="2"
        aria-hidden="true"
      />
    </span>
    {{ display }}
  </span>
</template>
