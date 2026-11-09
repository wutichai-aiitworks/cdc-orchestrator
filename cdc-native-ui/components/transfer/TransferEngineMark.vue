<script setup lang="ts">
// Logo for one end of a transfer. Uses the app's driver logos where they exist (Redis included);
// MongoDB and CSV have none in the app, so they get a neutral lettered tile rather than an invented mark.
import { FileArchive, Server } from "@lucide/vue"
import { dbIconPath } from "~/utils/dbIcon"

const props = withDefaults(defineProps<{ engine: string; size?: "sm" | "md" | "lg" }>(), { size: "md" })
const FALLBACK = dbIconPath("__none__")
const hasLogo = computed(() => dbIconPath(props.engine) !== FALLBACK)
const LETTERS: Record<string, string> = { mongodb: "Mo", mongo: "Mo", csv: "CSV" }
const NAMES: Record<string, string> = { mongodb: "MongoDB", mongo: "MongoDB", redis: "Redis", csv: "CSV file", sftp: "SFTP server", ssh: "SSH host", file: "File" }
const box = computed(() => ({ sm: "h-5 w-5 text-[8px] rounded", md: "h-7 w-7 text-[10px] rounded-md", lg: "h-10 w-10 text-xs rounded-lg" })[props.size])
const icon = computed(() => ({ sm: "h-3 w-3", md: "h-4 w-4", lg: "h-5 w-5" })[props.size])
</script>

<template>
  <AppDbIcon v-if="hasLogo" :type="engine" :size="size" />
  <span
    v-else
    class="flex shrink-0 items-center justify-center border border-border bg-muted font-semibold text-muted-foreground"
    :class="box"
    role="img"
    :aria-label="NAMES[engine] || engine"
  >
    <Server v-if="engine === 'sftp' || engine === 'ssh'" :class="icon" :stroke-width="1.75" />
    <FileArchive v-else-if="engine === 'file'" :class="icon" :stroke-width="1.75" />
    <template v-else>{{ LETTERS[engine] || engine.slice(0, 2) }}</template>
  </span>
</template>
