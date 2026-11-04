<script setup lang="ts">
// List of saved transfers with their last result. ↑/↓ move, Enter opens.
import { Plus, Search } from "@lucide/vue"
import type { Tone } from "~/utils/transferView"

export type RailItem = { id: string; name: string; route: string; status: { tone: Tone; text: string }; from: string }

const props = defineProps<{ items: RailItem[]; selected: string; purpose: string; newLabel?: string }>()
const emit = defineEmits<{ select: [id: string]; new: [] }>()
const q = ref("")
const filtered = computed(() => {
  const s = q.value.trim().toLowerCase()
  return s ? props.items.filter((i) => `${i.name} ${i.route}`.toLowerCase().includes(s)) : props.items
})
const list = ref<HTMLElement | null>(null)
const onKey = (e: KeyboardEvent) => {
  const i = filtered.value.findIndex((x) => x.id === props.selected)
  const next = e.key === "ArrowDown" ? i + 1 : e.key === "ArrowUp" ? i - 1 : -2
  if (next === -2 || !filtered.value.length) return
  e.preventDefault()
  const item = filtered.value[Math.min(filtered.value.length - 1, Math.max(0, next))]
  emit("select", item.id)
  nextTick(() => list.value?.querySelector<HTMLElement>(`[data-id="${item.id}"]`)?.focus())
}
const dot: Record<Tone, string> = {
  ok: "bg-emerald-500",
  warn: "bg-amber-500",
  err: "bg-red-500",
  run: "bg-primary animate-pulse motion-reduce:animate-none",
  idle: "bg-muted-foreground/40",
}
</script>

<template>
  <aside class="flex min-h-0 flex-col gap-3 lg:w-72 lg:shrink-0" aria-label="Transfers">
    <div class="flex items-start justify-between gap-2">
      <p class="text-xs leading-5 text-muted-foreground">{{ purpose }}</p>
      <AppButton size="sm" @click="emit('new')"><Plus class="h-3.5 w-3.5" :stroke-width="2" />{{ newLabel || "New" }}</AppButton>
    </div>
    <label v-if="items.length > 4" class="relative block">
      <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" :stroke-width="1.75" />
      <input v-model="q" type="search" class="auth-input w-full !pl-9" placeholder="Find a transfer" aria-label="Find a transfer" />
    </label>
    <div ref="list" class="flex flex-col gap-1 overflow-auto" role="listbox" aria-label="Saved transfers" @keydown="onKey">
      <button
        v-for="item in filtered"
        :key="item.id"
        :data-id="item.id"
        type="button"
        role="option"
        :aria-selected="selected === item.id"
        :tabindex="selected === item.id || (!selected && item === filtered[0]) ? 0 : -1"
        class="group flex w-full items-start gap-3 rounded-lg border px-3 py-2.5 text-left transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        :class="selected === item.id ? 'border-primary/50 bg-primary/5' : 'border-transparent hover:bg-muted/60'"
        @click="emit('select', item.id)"
      >
        <span class="relative mt-0.5 shrink-0">
          <TransferEngineMark :engine="item.from" size="sm" />
        </span>
        <span class="min-w-0 flex-1">
          <span class="block truncate text-sm font-medium text-foreground">{{ item.name }}</span>
          <span class="block truncate text-xs text-muted-foreground">{{ item.route }}</span>
          <span class="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground">
            <span class="h-1.5 w-1.5 shrink-0 rounded-full" :class="dot[item.status.tone]" aria-hidden="true" />
            <span class="truncate">{{ item.status.text }}</span>
          </span>
        </span>
      </button>
      <p v-if="items.length && !filtered.length" class="px-3 py-2 text-xs text-muted-foreground">No transfer matches “{{ q }}”.</p>
    </div>
  </aside>
</template>
