<script setup lang="ts">
import { formatApiError } from "~/utils/apiError"

const router = useRouter()
const toast = useToast()
const { createSchema } = useKafkaAdmin()

const subject = ref("")
const schemaType = ref("avro")
const compatibility = ref("")
const schema = ref("")
const submitting = ref(false)
const error = ref("")

const COMPAT_LEVELS = [
  { value: "", label: "Default (inherit global)" },
  { value: "BACKWARD", label: "BACKWARD" },
  { value: "BACKWARD_TRANSITIVE", label: "BACKWARD_TRANSITIVE" },
  { value: "FORWARD", label: "FORWARD" },
  { value: "FORWARD_TRANSITIVE", label: "FORWARD_TRANSITIVE" },
  { value: "FULL", label: "FULL" },
  { value: "FULL_TRANSITIVE", label: "FULL_TRANSITIVE" },
  { value: "NONE", label: "NONE" },
]

const SCHEMA_PLACEHOLDERS: Record<string, string> = {
  avro: `{
  "type": "record",
  "name": "MyRecord",
  "namespace": "com.example",
  "fields": [
    { "name": "id", "type": "long" },
    { "name": "name", "type": "string" }
  ]
}`,
  json: `{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "type": "object",
  "properties": {
    "id": { "type": "integer" },
    "name": { "type": "string" }
  },
  "required": ["id", "name"]
}`,
  protobuf: `syntax = "proto3";

message MyRecord {
  int64 id = 1;
  string name = 2;
}`,
}

const placeholder = computed(() => SCHEMA_PLACEHOLDERS[schemaType.value] || SCHEMA_PLACEHOLDERS.avro)

const submit = async () => {
  if (!subject.value.trim() || !schema.value.trim()) {
    error.value = "Subject and schema are required"
    return
  }
  submitting.value = true
  error.value = ""
  try {
    await createSchema({
      subject: subject.value.trim(),
      schema: schema.value,
      schema_type: schemaType.value,
      compatibility: compatibility.value || undefined,
    })
    toast.success(`Created schema ${subject.value.trim()}`)
    await router.push(`/schemas/${encodeURIComponent(subject.value.trim())}`)
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to create schema")
    toast.error(error.value)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div class="page-container">
    <AppPageHeader title="Create Schema" description="Register a new Avro, JSON Schema, or Protobuf subject.">
      <AppButton as="link" to="/schemas">Cancel</AppButton>
      <AppButton variant="primary" :disabled="submitting" @click="submit">
        {{ submitting ? "Creating…" : "Create Schema" }}
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" class="mb-4" />

    <div class="grid grid-cols-1 gap-5 lg:grid-cols-3">
      <!-- Left panel: metadata -->
      <div class="flex flex-col gap-5">
        <AppCard class="space-y-5">
          <h2 class="section-title">Subject</h2>

          <!-- Subject name -->
          <label>
            <span class="auth-label">Subject name <span class="text-red-400">*</span></span>
            <AppInput v-model="subject" placeholder="my-topic-value" />
            <span class="mt-1 block text-xs text-surface-400">
              Convention: <code class="font-mono">topic-key</code> or <code class="font-mono">topic-value</code>
            </span>
          </label>

          <!-- Schema type -->
          <div>
            <span class="auth-label">Schema type <span class="text-red-400">*</span></span>
            <div class="mt-1.5 flex gap-2">
              <button
                v-for="t in [
                  { value: 'avro', label: 'AVRO', color: 'border-blue-500 bg-blue-500/10 text-blue-400' },
                  { value: 'json', label: 'JSON', color: 'border-emerald-500 bg-emerald-500/10 text-emerald-400' },
                  { value: 'protobuf', label: 'PROTOBUF', color: 'border-purple-500 bg-purple-500/10 text-purple-400' },
                ]"
                :key="t.value"
                type="button"
                :class="[
                  'flex flex-1 items-center justify-center rounded-xl border px-3 py-2 text-xs font-semibold transition-all',
                  schemaType === t.value ? t.color : 'border-surface-700 bg-surface-800/50 text-surface-300 hover:border-surface-500',
                ]"
                @click="schemaType = t.value"
              >
                {{ t.label }}
              </button>
            </div>
          </div>

          <!-- Compatibility -->
          <label>
            <span class="auth-label">Compatibility level</span>
            <AppSelect v-model="compatibility">
              <option v-for="c in COMPAT_LEVELS" :key="c.value" :value="c.value">{{ c.label }}</option>
            </AppSelect>
            <span class="mt-1 block text-xs text-surface-400">
              Controls how future versions are validated.
            </span>
          </label>
        </AppCard>

        <!-- Quick reference -->
        <AppCard class="space-y-3">
          <h2 class="section-title">Quick reference</h2>
          <div class="space-y-2 text-xs text-surface-400">
            <div>
              <span class="font-semibold text-blue-400">AVRO</span> — binary format, strong typing, widely used with Kafka Connect.
            </div>
            <div>
              <span class="font-semibold text-emerald-400">JSON Schema</span> — human-readable, great for debugging. Larger payloads.
            </div>
            <div>
              <span class="font-semibold text-purple-400">Protobuf</span> — compact binary, language-neutral, requires proto file.
            </div>
          </div>
          <hr class="border-surface-700/50" />
          <div class="space-y-1 text-xs text-surface-400">
            <div class="font-semibold text-surface-300">Compatibility levels</div>
            <div><code class="font-mono text-brand-400">BACKWARD</code> — new schema reads old data</div>
            <div><code class="font-mono text-brand-400">FORWARD</code> — old schema reads new data</div>
            <div><code class="font-mono text-brand-400">FULL</code> — both directions</div>
            <div><code class="font-mono text-brand-400">NONE</code> — no checks</div>
          </div>
        </AppCard>
      </div>

      <!-- Right panel: schema editor -->
      <AppCard class="flex flex-col gap-3 lg:col-span-2">
        <h2 class="section-title">Schema definition <span class="text-red-400">*</span></h2>
        <p class="text-xs text-surface-400">Paste or write the full schema. The placeholder shows the expected format for the selected type.</p>
        <textarea
          v-model="schema"
          class="auth-input flex-1 font-mono text-sm"
          style="min-height: 520px; resize: vertical"
          spellcheck="false"
          :placeholder="placeholder"
        />
        <div class="flex justify-end">
          <AppButton variant="primary" :disabled="submitting" @click="submit">
            {{ submitting ? "Creating…" : "Create Schema" }}
          </AppButton>
        </div>
      </AppCard>
    </div>
  </div>
</template>
