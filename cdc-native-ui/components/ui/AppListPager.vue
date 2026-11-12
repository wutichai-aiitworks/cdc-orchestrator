<script setup lang="ts">
// Client-side list footer: "Showing a–b of n", rows-per-page select, Previous/Next.
// Hidden entirely when the list is empty; the page-size select stays visible on one page.
const props = withDefaults(
  defineProps<{
    total: number
    sizes?: number[]
    noun?: string
  }>(),
  { sizes: () => [12, 24, 48, 96], noun: "items" },
)

const page = defineModel<number>("page", { required: true })
const pageSize = defineModel<number>("pageSize", { required: true })

const totalPages = computed(() => Math.max(1, Math.ceil(props.total / pageSize.value)))
const first = computed(() => (props.total ? (page.value - 1) * pageSize.value + 1 : 0))
const last = computed(() => Math.min(props.total, page.value * pageSize.value))

// Filters can shrink the list under the current page; stay on the last real page.
watch(totalPages, (pages) => {
  if (page.value > pages) page.value = pages
})
</script>

<template>
  <div v-if="total" class="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-border pt-3">
    <div class="flex items-center gap-2 text-xs text-muted-foreground">
      <span>Showing {{ first }}–{{ last }} of {{ total }} {{ noun }}</span>
      <AppSelect
        :model-value="pageSize"
        class="w-28"
        aria-label="Rows per page"
        @update:model-value="pageSize = Number($event); page = 1"
      >
        <option v-for="size in sizes" :key="size" :value="size">{{ size }} / page</option>
      </AppSelect>
    </div>
    <AppPager
      v-if="totalPages > 1"
      :page="page"
      :total-pages="totalPages"
      @prev="page = Math.max(1, page - 1)"
      @next="page = Math.min(totalPages, page + 1)"
    />
  </div>
</template>
