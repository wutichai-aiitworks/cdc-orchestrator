<script setup lang="ts">
const { isDark } = useTheme()

useHead({
  title: "CDC API Reference · Hyper CDC WebUI",
})

const scalarUrl = computed(() => `/docs?theme=${isDark.value ? "dark" : "light"}`)
</script>

<template>
  <div class="page-container !max-w-none">
    <section class="page-toolbar">
      <div class="flex min-w-0 items-center gap-2">
        <h1 class="page-title">Hyper CDC API Reference</h1>
        <AppInfoHint text="Generated from docs/openapi.yaml" />
      </div>
      <div class="flex items-center gap-2">
        <a href="/openapi.json" download class="btn-secondary !w-auto">Download OpenAPI JSON</a>
      </div>
    </section>

    <ClientOnly>
      <iframe
        :key="scalarUrl"
        :src="scalarUrl"
        title="Hyper CDC API Reference"
        class="h-[calc(100vh-140px)] w-full rounded-xl border border-border bg-card"
      />
      <template #fallback>
        <div class="flex min-h-[calc(100vh-140px)] items-center justify-center text-sm text-muted-foreground">
          Loading API reference…
        </div>
      </template>
    </ClientOnly>
  </div>
</template>
