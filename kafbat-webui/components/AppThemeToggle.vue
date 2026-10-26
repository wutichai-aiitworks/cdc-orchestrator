<script setup lang="ts">
import { Moon, Sun } from "@lucide/vue"

const { isDark, toggle } = useTheme()
</script>

<template>
  <button
    type="button"
    class="icon-button"
    :title="isDark ? 'Switch to light mode' : 'Switch to dark mode'"
    :aria-label="isDark ? 'Switch to light mode' : 'Switch to dark mode'"
    @click="toggle()"
  >
    <Sun v-if="isDark" class="h-4 w-4" :stroke-width="1.75" />
    <Moon v-else class="h-4 w-4" :stroke-width="1.75" />
  </button>
</template>
