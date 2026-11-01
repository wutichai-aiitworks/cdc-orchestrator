<script setup lang="ts">
defineProps<{
  tableClass?: string
}>()
</script>

<template>
  <div class="overflow-auto rounded-xl">
    <table class="data-table" :class="tableClass">
      <slot />
    </table>
  </div>
</template>
