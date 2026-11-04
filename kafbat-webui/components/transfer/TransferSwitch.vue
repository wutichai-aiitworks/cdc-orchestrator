<script setup lang="ts">
// A setting described by its outcome. `danger` marks options that delete or replace data on
// the destination; the mark stays visible whether or not the option is on.
import { AlertTriangle } from "@lucide/vue"

const model = defineModel<boolean>({ default: false })
// `danger: true` reads "Replaces data on the target"; a string names a different risk.
const props = defineProps<{ label: string; help?: string; danger?: boolean | string; disabled?: boolean }>()
const dangerText = computed(() => (typeof props.danger === "string" ? props.danger : "Replaces data on the target"))
const id = useId()
</script>

<template>
  <div class="flex items-start gap-3" :class="disabled ? 'opacity-50' : ''">
    <button
      :id="id"
      type="button"
      role="switch"
      :aria-checked="model"
      :disabled="disabled"
      class="relative mt-0.5 inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed"
      :class="model ? (danger ? 'border-amber-500 bg-amber-500' : 'border-primary bg-primary') : 'border-border bg-muted'"
      @click="model = !model"
    >
      <span
        class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow transition-transform"
        :class="model ? 'translate-x-[1.1rem]' : 'translate-x-0.5'"
      />
    </button>
    <label :for="id" class="min-w-0 cursor-pointer" @click.prevent="!disabled && (model = !model)">
      <span class="block text-sm font-medium text-foreground">{{ label }}</span>
      <span v-if="help" class="block text-xs leading-5 text-muted-foreground">{{ help }}</span>
      <span v-if="danger" class="mt-0.5 inline-flex items-center gap-1 text-[11px] font-medium text-amber-600 dark:text-amber-400">
        <AlertTriangle class="h-3 w-3" :stroke-width="2" /> {{ dangerText }}
      </span>
    </label>
  </div>
</template>
