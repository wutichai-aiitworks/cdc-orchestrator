<script setup lang="ts">
// Appears only when the editor differs from the saved transfer. Lists what changed so a save is
// never a surprise; Run stays disabled while this bar is showing.
defineProps<{ changes: string[]; saving?: boolean; isNew?: boolean }>()
defineEmits<{ save: []; discard: [] }>()
</script>

<template>
  <div
    v-if="changes.length"
    class="sticky bottom-0 z-10 -mx-1 mt-2 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-amber-500/40 bg-card/95 px-4 py-3 shadow-lg backdrop-blur"
    role="region"
    aria-label="Unsaved changes"
  >
    <div class="flex min-w-0 items-center gap-2 text-sm">
      <span class="h-2 w-2 shrink-0 rounded-full bg-amber-500" aria-hidden="true" />
      <span v-if="isNew" class="text-foreground">New transfer, not saved yet</span>
      <span v-else class="truncate text-foreground">
        {{ changes.length === 1 ? "1 unsaved change" : `${changes.length} unsaved changes` }}:
        <span class="text-muted-foreground">{{ changes.join(", ") }}</span>
      </span>
    </div>
    <div class="ml-auto flex shrink-0 gap-2">
      <AppButton variant="ghost" size="sm" :disabled="saving" @click="$emit('discard')">{{ isNew ? "Cancel" : "Discard" }}</AppButton>
      <AppButton variant="primary" size="sm" :disabled="saving" @click="$emit('save')">
        {{ saving ? "Saving…" : isNew ? "Create transfer" : "Save changes" }}
      </AppButton>
    </div>
  </div>
</template>
