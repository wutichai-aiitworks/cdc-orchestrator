<script setup lang="ts">
// 45° corner ribbon for APP_MODE: amber "Maintenance", violet "Enhanced", nothing in Production.
// Clicks pass through; the header moves its right-hand controls clear while a ribbon shows.
const { mode, ribbon } = useAppMode()
// Colour classes stay in this .vue file so Tailwind sees them.
const COLOUR: Record<string, string> = { maintenance: "border-amber-700/30 bg-amber-500", enhanced: "border-violet-800/30 bg-violet-600" }
</script>

<template>
  <div v-if="ribbon" class="pointer-events-none fixed right-0 top-0 z-[70] h-28 w-28 overflow-hidden" role="status" :aria-label="`${ribbon.label} mode`">
    <div
      class="absolute right-[-46px] top-[22px] w-[180px] rotate-45 border-y py-1 text-center text-[11px] font-bold uppercase tracking-[0.18em] text-white"
      :class="COLOUR[mode]"
      style="background-image: repeating-linear-gradient(-45deg, rgb(0 0 0 / 0.08) 0 6px, transparent 6px 12px)"
      :title="ribbon.title"
    >
      {{ ribbon.label }}
    </div>
  </div>
</template>
