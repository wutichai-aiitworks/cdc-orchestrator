<!-- In layouts/default.vue: script -->
<script setup lang="ts">
const { ribbon: modeRibbon } = useAppMode()
</script>

<!-- In the header: wrap the right-hand controls (theme toggle, avatar) -->
<template>
  <div class="flex items-center gap-2" :class="modeRibbon ? 'mr-16 md:mr-14' : ''">
    <!-- theme button, avatar menu … -->
  </div>
</template>
