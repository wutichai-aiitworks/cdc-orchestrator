<script setup lang="ts">
import { AlertTriangle, Check, Info, X, XCircle } from "@lucide/vue"

const { toasts, remove } = useToast()

const icons = {
  success: Check,
  error: XCircle,
  info: Info,
  warning: AlertTriangle,
}

// Light mode: soft tinted background + dark text for contrast.
// Dark mode:  deep background + bright tinted text.
const colors = {
  success: "border-emerald-200 bg-emerald-50  text-emerald-800 dark:border-emerald-500/40 dark:bg-emerald-950 dark:text-emerald-300",
  error:   "border-red-200     bg-red-50      text-red-800     dark:border-red-500/40     dark:bg-red-950     dark:text-red-300",
  info:    "border-blue-200    bg-blue-50     text-blue-800    dark:border-blue-500/40    dark:bg-blue-950    dark:text-blue-300",
  warning: "border-yellow-200  bg-yellow-50   text-yellow-800  dark:border-yellow-500/40  dark:bg-yellow-950  dark:text-yellow-300",
}
</script>

<template>
  <Teleport to="body">
    <div class="fixed right-4 top-4 z-[1200] flex w-[calc(100%-2rem)] max-w-sm flex-col gap-2">
      <TransitionGroup name="toast">
        <div
          v-for="toast in toasts"
          :key="toast.id"
          :class="[
            'flex items-center gap-3 rounded-xl border px-4 py-3 shadow-2xl',
            colors[toast.type],
          ]"
          role="status"
        >
          <component :is="icons[toast.type]" class="h-4 w-4 flex-shrink-0" :stroke-width="1.75" />
          <p class="flex-1 text-sm font-medium">{{ toast.message }}</p>
          <button
            type="button"
            class="rounded-lg p-1 opacity-50 transition-opacity hover:opacity-100 focus:outline-none focus:ring-1 focus:ring-current"
            title="Dismiss"
            aria-label="Dismiss notification"
            @click="remove(toast.id)"
          >
            <X class="h-3.5 w-3.5" :stroke-width="1.75" />
          </button>
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>

<style scoped>
.toast-enter-active,
.toast-leave-active {
  transition: all 0.3s ease;
}

.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateX(100%);
}
</style>
