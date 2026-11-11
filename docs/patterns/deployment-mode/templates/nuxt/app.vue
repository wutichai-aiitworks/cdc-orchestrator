<script setup lang="ts">
import type { AppMode } from "~/utils/appMode"
// Loaded during server rendering so the ribbon is in the first HTML (no flash after load).
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
