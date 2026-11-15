<script setup lang="ts">
import type { AppMode } from "~/utils/appMode"
// APP_MODE comes from the UI server's environment; loaded while rendering on the server so the
// mode ribbon is there on first paint (no flash after load).
const { mode } = useAppMode()
// useAsyncData + a plain fetch (same SSR/hydration behaviour as useFetch). Route typing is kept
// cheap in nuxt.config.ts (nitro "types:extend"), which is what used to hit TS2589.
// useRequestFetch forwards the request's cookies/headers during SSR, as useFetch does.
const modeUrl: string = "/api/ui/mode"
const requestFetch = useRequestFetch() as unknown as <T>(url: string) => Promise<T>
const { data } = await useAsyncData("app-mode", () => requestFetch<{ mode: AppMode }>(modeUrl))
if (data.value?.mode) mode.value = data.value.mode
</script>

<template>
  <NuxtLayout>
    <NuxtPage />
  </NuxtLayout>
  <AppModeRibbon />
</template>
