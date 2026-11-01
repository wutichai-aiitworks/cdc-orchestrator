<script setup lang="ts">
import { dbIconPath } from "~/utils/dbIcon"

const props = withDefaults(defineProps<{
  type: string | null | undefined
  size?: "sm" | "md" | "lg"
}>(), { size: "md" })

const src = computed(() => dbIconPath(props.type))

const sizeClass = computed(() => ({
  sm: "h-5 w-5",
  md: "h-7 w-7",
  lg: "h-10 w-10",
}[props.size]))
</script>

<template>
  <img
    :src="src"
    :alt="type ?? 'database'"
    :class="[sizeClass, 'object-contain flex-shrink-0']"
  />
</template>
