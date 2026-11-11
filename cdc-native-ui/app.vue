<script setup lang="ts">
import type { AppMode } from "~/utils/appMode"
// APP_MODE comes from the UI server's environment; loaded while rendering on the server so the
// mode ribbon is there on first paint (no flash after load).
const { mode } = useAppMode()
const { data } = await useFetch<{ mode: AppMode }>("/api/ui/mode", { key: "app-mode" })
if (data.value?.mode) mode.value = data.value.mode
</script>

<template>
  <NuxtLayout>
    <NuxtPage />
  </NuxtLayout>
  <AppModeRibbon />
</template>
