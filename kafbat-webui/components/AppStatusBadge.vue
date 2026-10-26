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
  if (has("ok", "success", "enabled", "ready", "resolved", "valid", "connected", "configured")) {
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

const display = computed(() => props.label || props.status || "Unknown")
</script>

<template>
  <span :class="visual.tone" role="status" :title="display">
    <span class="relative inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center">
      <component
        :is="visual.icon"
        class="relative h-3.5 w-3.5"
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
