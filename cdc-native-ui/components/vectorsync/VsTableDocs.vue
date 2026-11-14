<script setup lang="ts">
// How one table's rows become documents: which columns are the text that gets embedded, which
// travel along as filterable metadata, and which never leave the database (sensitive by default).
import { EyeOff, FileText, Scissors, ShieldAlert, Wand2 } from "@lucide/vue"
import { DEFAULT_CHUNKING, FILE_COLUMN_RE, SENSITIVE_RE, suggestTemplate, templateError, unknownTemplateFields, type ContentMode, type TableDraft } from "~/utils/vectorSync"

const props = defineProps<{ table: string; columns: string[]; modelValue: TableDraft; docling?: { available: boolean; version?: string | null; max_mb?: number } | null }>()
const emit = defineEmits<{ "update:modelValue": [v: TableDraft] }>()

// Drafts saved before Docling/chunking existed lack these fields.
const t = computed<TableDraft>(() => {
  const v = props.modelValue as Partial<TableDraft> & Pick<TableDraft, "mode" | "include_fields" | "exclude_fields" | "metadata_fields" | "content_template">
  return { document_column: "", filename_column: "", include_row_text: false, ...v, chunking: { ...DEFAULT_CHUNKING, ...(v.chunking || {}) } }
})
const set = (patch: Partial<TableDraft>) => emit("update:modelValue", { ...t.value, ...patch })
const toggle = (list: string[], c: string) => (list.includes(c) ? list.filter((x) => x !== c) : [...list, c])

const MODES: { key: ContentMode; label: string; hint: string }[] = [
  { key: "all", label: "All columns", hint: "Every column as “name: value” lines. Good first choice." },
  { key: "pick", label: "Pick columns", hint: "Only the columns that carry meaning — shorter text, better matches." },
  { key: "template", label: "Template", hint: "Write the sentence yourself, e.g. “Order $id for ${customer}: $note”." },
  { key: "document", label: "Document (Docling)", hint: "A column holds a PDF, Word, PowerPoint, Excel or HTML file. Docling reads it and splits it by sections and tables." },
]
const doclingOn = computed(() => !!props.docling?.available)
const fileColumns = computed(() => [...props.columns].sort((a, b) => Number(FILE_COLUMN_RE.test(b)) - Number(FILE_COLUMN_RE.test(a))))
const setChunk = (patch: Partial<TableDraft["chunking"]>) => set({ chunking: { ...t.value.chunking, ...patch } })
const pickMode = (key: ContentMode) => {
  if (key === "document" && !doclingOn.value) return
  const patch: Partial<TableDraft> = { mode: key }
  if (key === "template" && !t.value.content_template) patch.content_template = suggestTemplate(props.columns, t.value.exclude_fields)
  if (key === "document" && !t.value.document_column) patch.document_column = fileColumns.value.find((c) => FILE_COLUMN_RE.test(c)) || ""
  set(patch)
}

const visible = computed(() => props.columns.filter((c) => !t.value.exclude_fields.includes(c)))
const sensitive = computed(() => props.columns.filter((c) => SENSITIVE_RE.test(c)))
const tplErr = computed(() => (t.value.mode === "template" ? templateError(t.value.content_template) : null))
const unknown = computed(() => (t.value.mode === "template" ? unknownTemplateFields(t.value.content_template, props.columns) : []))

const tpl = ref<HTMLTextAreaElement | null>(null)
const insert = (c: string) => {
  const el = tpl.value
  const token = `\${${c}}`
  const text = t.value.content_template
  const at = el ? el.selectionStart : text.length
  const end = el ? el.selectionEnd : text.length
  set({ content_template: text.slice(0, at) + token + text.slice(end) })
  nextTick(() => {
    el?.focus()
    el?.setSelectionRange(at + token.length, at + token.length)
  })
}
const suggest = () => set({ content_template: suggestTemplate(props.columns, t.value.exclude_fields) })

const metaAll = computed(() => t.value.metadata_fields === null)
const setMetaAll = (all: boolean) => set({ metadata_fields: all ? null : [...visible.value] })
</script>

<template>
  <div class="space-y-5">
    <!-- content mode -->
    <div>
      <p class="mb-2 text-xs font-medium text-muted-foreground">Text to embed</p>
      <div class="grid gap-2 sm:grid-cols-2 xl:grid-cols-4" role="radiogroup">
        <button
          v-for="m in MODES"
          :key="m.key"
          type="button"
          role="radio"
          :aria-checked="t.mode === m.key"
          class="rounded-lg border px-3 py-2 text-left transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
          :class="t.mode === m.key ? 'border-primary bg-brand-500/10' : 'border-border hover:bg-muted'"
          :disabled="m.key === 'document' && !doclingOn"
          :title="m.key === 'document' && !doclingOn ? 'Docling is not in this sidecar image: set VECTORSYNC_DOCLING=true in docker/.env and redeploy' : undefined"
          @click="pickMode(m.key)"
        >
          <p class="text-sm font-medium" :class="t.mode === m.key ? 'text-primary' : 'text-foreground'">{{ m.label }}</p>
          <p class="mt-0.5 text-xs text-muted-foreground">{{ m.hint }}</p>
          <p v-if="m.key === 'document' && !doclingOn" class="mt-1 text-[11px] text-amber-700 dark:text-amber-400">Needs VECTORSYNC_DOCLING=true</p>
        </button>
      </div>
    </div>

    <!-- document (Docling) -->
    <div v-if="t.mode === 'document'" class="space-y-3 rounded-lg border border-border p-3">
      <div class="flex items-center gap-2 text-xs font-medium text-muted-foreground">
        <FileText class="h-4 w-4" :stroke-width="1.75" />Docling {{ docling?.version || "" }} · files up to {{ docling?.max_mb || 50 }} MB · OCR off
      </div>
      <div class="grid gap-3 md:grid-cols-2">
        <label class="block">
          <AppFormLabel label="Column with the file" hint="bytea / BLOB / VARBINARY, or base64 text. It is converted, never stored as text or metadata." />
          <select :value="t.document_column" class="auth-input font-mono" @change="set({ document_column: ($event.target as HTMLSelectElement).value })">
            <option value="">— choose —</option>
            <option v-for="c in fileColumns" :key="c" :value="c">{{ c }}</option>
          </select>
        </label>
        <label class="block">
          <AppFormLabel label="File name column (optional)" hint="Helps detect the format; otherwise it is read from the file itself." />
          <select :value="t.filename_column" class="auth-input font-mono" @change="set({ filename_column: ($event.target as HTMLSelectElement).value })">
            <option value="">— none —</option>
            <option v-for="c in columns.filter((x) => x !== t.document_column)" :key="c" :value="c">{{ c }}</option>
          </select>
        </label>
      </div>
      <label class="flex items-start gap-2 text-xs">
        <input type="checkbox" class="mt-0.5 h-4 w-4 rounded border-border accent-brand-500" :checked="t.include_row_text" @change="set({ include_row_text: ($event.target as HTMLInputElement).checked })" />
        <span><span class="font-medium text-foreground">Add the row's other columns to every chunk</span><br /><span class="text-muted-foreground">e.g. product name or customer, so a chunk is found by them too. Uses the columns not excluded below.</span></span>
      </label>
      <p class="text-[11px] text-muted-foreground">Kafka messages carry the whole file: files over ~1 MB need larger message limits on Connect and the broker.</p>
    </div>

    <p v-if="!columns.length" class="alert-info text-xs">
      Columns appear once the preview has read a record from this table's topic.
    </p>

    <!-- columns: exclude (all) or include (pick) -->
    <div v-if="columns.length && t.mode !== 'template'">
      <p class="mb-2 text-xs font-medium text-muted-foreground">
        {{ t.mode === "pick" ? "Columns in the text — click to add or remove" : "Click a column to leave it out" }}
      </p>
      <div class="flex flex-wrap gap-1.5">
        <button
          v-for="c in columns.filter((x) => !(t.mode === 'document' && x === t.document_column))"
          :key="c"
          type="button"
          class="inline-flex items-center gap-1 rounded-md border px-2 py-1 font-mono text-xs transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          :class="[
            t.exclude_fields.includes(c)
              ? 'border-dashed border-border text-muted-foreground line-through'
              : t.mode === 'pick' && !t.include_fields.includes(c)
                ? 'border-border text-muted-foreground hover:bg-muted'
                : 'border-primary/40 bg-brand-500/10 text-primary',
          ]"
          :disabled="t.mode === 'pick' && t.exclude_fields.includes(c)"
          @click="t.mode === 'pick' ? set({ include_fields: toggle(t.include_fields, c) }) : set({ exclude_fields: toggle(t.exclude_fields, c) })"
        >
          <ShieldAlert v-if="SENSITIVE_RE.test(c)" class="h-3 w-3" :stroke-width="1.75" />
          {{ c }}
        </button>
      </div>
    </div>

    <!-- template -->
    <div v-if="t.mode === 'template'">
      <div class="mb-2 flex items-center justify-between gap-2">
        <p class="text-xs font-medium text-muted-foreground">Template — <span class="font-mono">$column</span> or <span class="font-mono">${column}</span>; <span class="font-mono">$$</span> is a dollar sign</p>
        <button type="button" class="inline-flex h-7 items-center gap-1.5 rounded-lg px-2 text-xs font-medium text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-50" :disabled="!columns.length" @click="suggest"><Wand2 class="h-3.5 w-3.5" :stroke-width="1.75" />Suggest</button>
      </div>
      <textarea
        ref="tpl"
        :value="t.content_template"
        rows="4"
        spellcheck="false"
        class="auth-input font-mono text-xs"
        :class="tplErr || unknown.length ? 'border-destructive' : ''"
        placeholder="Order ${id} for ${customer_name}: ${note}"
        @input="set({ content_template: ($event.target as HTMLTextAreaElement).value })"
      />
      <p v-if="tplErr" class="mt-1 text-xs text-destructive">{{ tplErr }}</p>
      <p v-else-if="unknown.length" class="mt-1 text-xs text-destructive">No column named {{ unknown.map((u) => "$" + u).join(", ") }} — every row would fail.</p>
      <div v-if="visible.length" class="mt-2 flex flex-wrap gap-1.5">
        <button v-for="c in visible" :key="c" type="button" class="rounded-md border border-border px-2 py-0.5 font-mono text-[11px] text-muted-foreground hover:bg-muted hover:text-foreground" @click="insert(c)">+ {{ c }}</button>
      </div>
    </div>

    <p v-if="sensitive.length" class="flex items-start gap-2 text-xs text-muted-foreground">
      <EyeOff class="mt-0.5 h-3.5 w-3.5 shrink-0" :stroke-width="1.75" />
      <span>
        Columns that look like credentials start out excluded (<span class="font-mono">{{ sensitive.join(", ") }}</span>). Excluded columns are never sent to the
        embedding model or stored as metadata.
      </span>
    </p>

    <!-- chunking -->
    <div class="rounded-lg border border-border p-3">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <p class="flex items-center gap-2 text-xs font-medium text-muted-foreground"><Scissors class="h-4 w-4" :stroke-width="1.75" />Split into chunks</p>
        <label v-if="t.mode !== 'document'" class="inline-flex items-center gap-2 text-xs text-muted-foreground">
          <input type="checkbox" class="h-4 w-4 rounded border-border accent-brand-500" :checked="t.chunking.enabled" @change="setChunk({ enabled: ($event.target as HTMLInputElement).checked })" />
          Long rows become several vectors
        </label>
        <span v-else class="text-xs text-muted-foreground">Always on for documents</span>
      </div>
      <div v-if="t.mode === 'document' || t.chunking.enabled" class="mt-3 grid gap-3 sm:grid-cols-2">
        <label class="block">
          <AppFormLabel label="Chunk size (characters)" hint="~4 characters per token. 800–2000 suits most embedding models." />
          <input type="number" min="200" max="20000" step="100" class="auth-input" :value="t.chunking.max_chars" @input="setChunk({ max_chars: Number(($event.target as HTMLInputElement).value) || 0 })" />
        </label>
        <label class="block">
          <AppFormLabel label="Overlap (characters)" hint="Repeated from the previous chunk so a sentence split in two is found from both." />
          <input type="number" min="0" step="50" class="auth-input" :value="t.chunking.overlap" @input="setChunk({ overlap: Number(($event.target as HTMLInputElement).value) || 0 })" />
        </label>
      </div>
      <p v-if="t.mode === 'document' || t.chunking.enabled" class="mt-2 text-[11px] text-muted-foreground">
        Each chunk is its own vector (<span class="font-mono">id#c0, #c1…</span>) with <span class="font-mono">_parent_id</span> = the row; an update replaces all chunks of the row.
      </p>
    </div>

    <!-- metadata -->
    <div v-if="columns.length">
      <div class="mb-2 flex items-center justify-between gap-2">
        <p class="text-xs font-medium text-muted-foreground">
          Metadata (stored next to each vector, usable as search filters)
        </p>
        <label class="inline-flex items-center gap-2 text-xs text-muted-foreground">
          <input type="checkbox" :checked="metaAll" class="h-4 w-4 rounded border-border accent-brand-500" @change="setMetaAll(($event.target as HTMLInputElement).checked)" />
          All included columns
        </label>
      </div>
      <div v-if="!metaAll" class="flex flex-wrap gap-1.5">
        <button
          v-for="c in visible"
          :key="c"
          type="button"
          class="rounded-md border px-2 py-0.5 font-mono text-xs"
          :class="t.metadata_fields?.includes(c) ? 'border-primary/40 bg-brand-500/10 text-primary' : 'border-border text-muted-foreground hover:bg-muted'"
          @click="set({ metadata_fields: toggle(t.metadata_fields || [], c) })"
        >
          {{ c }}
        </button>
      </div>
    </div>
  </div>
</template>
