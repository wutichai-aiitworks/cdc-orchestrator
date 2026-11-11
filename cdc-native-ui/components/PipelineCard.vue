<script setup lang="ts">
/** One pipeline as a card (card view and tag view of the pipelines list). */
import { ArrowRight, Pencil, Trash2, Waves } from "@lucide/vue"
import type { CdcProfile } from "~/composables/useProfilesM3"

defineProps<{
  profile: CdcProfile
  route: string
  capture: string
  pattern: string
  selected?: boolean
}>()

const emit = defineEmits<{ select: []; edit: []; delete: []; tag: [tag: string] }>()
</script>

<template>
  <article
    class="group flex cursor-pointer flex-col gap-3 rounded-xl border bg-card p-4 transition-colors hover:border-brand-500/50"
    :class="selected ? 'border-brand-500 bg-brand-500/5' : 'border-border'"
    tabindex="0"
    @click="emit('select')"
    @keydown.enter="emit('select')"
  >
    <header class="flex items-start justify-between gap-2">
      <div class="min-w-0">
        <p class="truncate font-medium text-foreground" :title="profile.name">{{ profile.name }}</p>
        <p v-if="profile.description" class="mt-0.5 line-clamp-2 text-xs text-muted-foreground">{{ profile.description }}</p>
      </div>
      <AppStatusBadge :status="profile.status" />
    </header>

    <div class="flex items-center gap-1.5">
      <AppDbIcon :type="profile.source.source_type" size="sm" class="opacity-90" />
      <ArrowRight class="h-3 w-3 shrink-0 text-muted-foreground" :stroke-width="1.75" />
      <AppDbIcon
        v-if="profile.pattern === 'source_to_target'"
        :type="profile.target?.target_type || 'postgres'"
        size="sm"
        class="opacity-90"
      />
      <Waves v-else class="h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
      <span class="ml-1 truncate font-mono text-xs text-muted-foreground" :title="route">{{ route }}</span>
    </div>

    <dl class="grid grid-cols-2 gap-2 text-xs">
      <div>
        <dt class="text-muted-foreground">Capture</dt>
        <dd class="text-foreground">{{ capture }}</dd>
      </div>
      <div>
        <dt class="text-muted-foreground">Pattern</dt>
        <dd class="text-foreground">{{ pattern }}</dd>
      </div>
    </dl>

    <footer class="mt-auto flex items-end justify-between gap-2">
      <div class="flex min-w-0 flex-wrap gap-1">
        <button
          v-for="tag in profile.tags ?? []"
          :key="tag"
          type="button"
          class="tag-chip hover:bg-brand-500/20"
          :title="`Show only '${tag}'`"
          @click.stop="emit('tag', tag)"
        >
          {{ tag }}
        </button>
      </div>
      <div class="flex shrink-0 gap-1" @click.stop>
        <button type="button" class="icon-button h-8 w-8" :title="profile.status === 'draft' ? 'Continue setup' : 'Edit'" @click="emit('edit')">
          <Pencil class="h-4 w-4" :stroke-width="1.75" />
        </button>
        <button type="button" class="icon-button h-8 w-8 text-red-500" title="Delete" @click="emit('delete')">
          <Trash2 class="h-4 w-4" :stroke-width="1.75" />
        </button>
      </div>
    </footer>
  </article>
</template>
