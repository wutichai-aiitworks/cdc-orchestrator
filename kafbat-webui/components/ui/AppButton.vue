<script setup lang="ts">
const props = withDefaults(
  defineProps<{
    as?: "button" | "link"
    to?: string
    variant?: "primary" | "secondary" | "ghost" | "destructive"
    size?: "default" | "sm" | "icon"
    disabled?: boolean
    title?: string
  }>(),
  {
    as: "button",
    variant: "secondary",
    size: "default",
    disabled: false,
    title: undefined,
  }
)

const klass = computed(() => {
  const variants = {
    primary: "btn-primary",
    secondary: "btn-secondary",
    ghost:
      "inline-flex h-10 min-h-10 items-center justify-center gap-2 rounded-xl px-4 text-sm font-medium text-muted-foreground hover:bg-accent hover:text-foreground focus:outline-none focus:ring-1 focus:ring-ring disabled:pointer-events-none disabled:opacity-50",
    destructive:
      "inline-flex h-10 min-h-10 items-center justify-center gap-2 rounded-xl bg-destructive px-4 text-sm font-semibold text-destructive-foreground hover:brightness-95 focus:outline-none focus:ring-1 focus:ring-ring disabled:pointer-events-none disabled:opacity-50",
  }
  return [
    variants[props.variant],
    props.size === "icon"
      ? "!h-9 !min-h-9 !w-9 !px-0"
      : props.size === "sm"
        ? "!h-8 !min-h-8 !w-auto !px-3 !text-xs"
        : "!w-auto",
  ]
})
</script>

<template>
  <NuxtLink
    v-if="as === 'link'"
    :to="to || '/'"
    :class="klass"
    :title="title"
    :aria-disabled="disabled"
    :tabindex="disabled ? -1 : undefined"
  >
    <slot />
  </NuxtLink>
  <button v-else type="button" :disabled="disabled" :class="klass" :title="title">
    <slot />
  </button>
</template>
