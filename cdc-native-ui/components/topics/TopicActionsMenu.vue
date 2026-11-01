<script setup lang="ts">
import { MoreVertical } from "@lucide/vue"
import type { TopicItem } from "~/composables/useKafkaM1"

const props = defineProps<{
  topic: TopicItem
  disabled?: boolean
}>()

const emit = defineEmits<{
  clear: [topic: TopicItem]
  recreate: [topic: TopicItem]
  remove: [topic: TopicItem]
}>()

const open = ref(false)
const btnRef = ref<HTMLElement | null>(null)
const menuStyle = ref({ top: "0px", left: "0px" })

const canMutate = computed(() => !props.disabled && !props.topic.internal)
const cleanupIsDelete = computed(
  () => (props.topic.cleanup_policy || "delete").toLowerCase() === "delete"
)

const toggleMenu = () => {
  if (props.topic.internal) return
  if (!open.value) {
    const rect = btnRef.value?.getBoundingClientRect()
    if (rect) {
      menuStyle.value = {
        top: `${rect.bottom + 4}px`,
        left: `${Math.max(8, rect.right - 220)}px`,
      }
    }
  }
  open.value = !open.value
}

const run = (action: "clear" | "recreate" | "remove") => {
  open.value = false
  if (action === "clear") emit("clear", props.topic)
  else if (action === "recreate") emit("recreate", props.topic)
  else emit("remove", props.topic)
}

const closeMenu = () => {
  open.value = false
}

onMounted(() => window.addEventListener("scroll", closeMenu, true))
onUnmounted(() => window.removeEventListener("scroll", closeMenu, true))
</script>

<template>
  <div class="flex justify-end">
    <button
      ref="btnRef"
      type="button"
      class="inline-flex h-8 w-8 items-center justify-center rounded-lg transition-colors"
      :class="
        topic.internal
          ? 'cursor-not-allowed text-muted-foreground/25'
          : disabled
            ? 'cursor-not-allowed text-muted-foreground/50'
            : 'text-muted-foreground hover:bg-accent hover:text-foreground'
      "
      :title="topic.internal ? 'Internal topics cannot be modified' : disabled ? 'Loading…' : 'Topic actions'"
      @click="toggleMenu"
    >
      <MoreVertical class="h-4 w-4" :stroke-width="1.75" />
    </button>

    <Teleport to="body">
      <div
        v-if="open && canMutate"
        class="fixed z-[9999] min-w-[220px] rounded-xl border border-border bg-popover p-1 shadow-2xl"
        :style="menuStyle"
        @click.stop
      >
        <!-- backdrop to close on outside-click -->
        <div class="fixed inset-0 -z-10" @click="closeMenu" />

        <button
          type="button"
          class="flex w-full flex-col items-start rounded-lg px-3 py-2 text-left text-sm text-red-500 hover:bg-red-500/10 disabled:cursor-not-allowed disabled:opacity-40"
          :disabled="!cleanupIsDelete"
          @click="run('clear')"
        >
          Clear Messages
          <span class="mt-0.5 text-xs font-normal text-muted-foreground">
            Only for topics with DELETE cleanup policy
          </span>
        </button>

        <button
          type="button"
          class="w-full rounded-lg px-3 py-2 text-left text-sm text-red-500 hover:bg-red-500/10"
          @click="run('recreate')"
        >
          Recreate Topic
        </button>

        <NuxtLink
          :to="`/topics/${encodeURIComponent(topic.name)}?tab=messages`"
          class="block w-full rounded-lg px-3 py-2 text-left text-sm hover:bg-accent"
          @click="closeMenu"
        >
          Produce Message
        </NuxtLink>

        <div class="my-1 border-t border-border/60" />

        <button
          type="button"
          class="w-full rounded-lg px-3 py-2 text-left text-sm text-red-500 hover:bg-red-500/10"
          @click="run('remove')"
        >
          Remove Topic
        </button>
      </div>
    </Teleport>
  </div>
</template>
