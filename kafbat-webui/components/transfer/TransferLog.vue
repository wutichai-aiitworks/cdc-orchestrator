<script setup lang="ts">
// Log tail with follow, wrap and copy. error:/warning: lines are highlighted so the line that
// matters is found without reading the whole log.
import { Copy, Maximize2, Minimize2, WrapText } from "@lucide/vue"
import { stripAnsi } from "~/utils/transferView"

const props = withDefaults(defineProps<{ text: string; live?: boolean; title?: string }>(), { live: false, title: "Log" })
const toast = useToast()
const wrap = ref(true)
const expanded = ref(false)
const box = ref<HTMLElement | null>(null)

const lines = computed(() =>
  stripAnsi(props.text || "").replace(/\n$/, "").split("\n").map((t) => ({
    t,
    tone: /\berror\b|failed|fatal/i.test(t) ? "err" : /\bwarning\b|not pinned/i.test(t) ? "warn" : /^---/.test(t) ? "mark" : "",
  })),
)

const atBottom = () => {
  const el = box.value
  return !el || el.scrollHeight - el.scrollTop - el.clientHeight < 40
}
watch(
  () => props.text,
  async () => {
    const follow = atBottom()
    await nextTick()
    if (follow && box.value) box.value.scrollTop = box.value.scrollHeight
  },
  { immediate: true },
)

const copy = async () => {
  try {
    await navigator.clipboard.writeText(stripAnsi(props.text || ""))
    toast.success("Log copied")
  } catch {
    toast.error("Copy failed; select the text instead")
  }
}
</script>

<template>
  <div class="overflow-hidden rounded-lg border border-border bg-muted/40">
    <div class="flex items-center justify-between gap-2 border-b border-border px-3 py-1.5">
      <span class="text-xs font-medium text-muted-foreground">
        {{ title }}<template v-if="live"> · following</template>
      </span>
      <span class="flex items-center gap-1">
        <AppButton variant="ghost" size="icon" :title="wrap ? 'Do not wrap lines' : 'Wrap lines'" @click="wrap = !wrap">
          <WrapText class="h-4 w-4" :stroke-width="1.75" />
        </AppButton>
        <AppButton variant="ghost" size="icon" title="Copy log" @click="copy"><Copy class="h-4 w-4" :stroke-width="1.75" /></AppButton>
        <AppButton variant="ghost" size="icon" :title="expanded ? 'Show less' : 'Show more'" @click="expanded = !expanded">
          <component :is="expanded ? Minimize2 : Maximize2" class="h-4 w-4" :stroke-width="1.75" />
        </AppButton>
      </span>
    </div>
    <div
      ref="box"
      class="overflow-auto px-3 py-2 font-mono text-xs leading-5"
      :class="[expanded ? 'max-h-[70vh]' : 'max-h-64', wrap ? 'whitespace-pre-wrap break-all' : 'whitespace-pre']"
      tabindex="0"
      aria-label="Log output"
    >
      <p v-if="!text" class="font-sans italic text-muted-foreground">No output yet.</p>
      <div
        v-for="(l, i) in lines"
        v-else
        :key="i"
        :class="{
          'text-red-600 dark:text-red-400': l.tone === 'err',
          'text-amber-600 dark:text-amber-400': l.tone === 'warn',
          'text-foreground font-medium': l.tone === 'mark',
          'text-muted-foreground': !l.tone,
        }"
      >{{ l.t || " " }}</div>
    </div>
  </div>
</template>
