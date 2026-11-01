<script setup lang="ts">
import { formatApiError } from "~/utils/apiError"
import { formatDurationMs } from "~/utils/bytes"

const router = useRouter()
const toast = useToast()
const { createTopic } = useKafkaM1()

const TOPIC_NAME_PATTERN = /^[a-zA-Z0-9._-]+$/
const BYTES_IN_GB = 1_073_741_824
const MILLISECONDS_IN_HOUR = 3_600_000
const MILLISECONDS_IN_DAY = 86_400_000
const MILLISECONDS_IN_WEEK = 604_800_000

const RETENTION_PRESETS = [
  { label: "1 hour", ms: MILLISECONDS_IN_HOUR },
  { label: "3 hours", ms: 3 * MILLISECONDS_IN_HOUR },
  { label: "6 hours", ms: 6 * MILLISECONDS_IN_HOUR },
  { label: "12 hours", ms: 12 * MILLISECONDS_IN_HOUR },
  { label: "1 day", ms: MILLISECONDS_IN_DAY },
  { label: "2 days", ms: 2 * MILLISECONDS_IN_DAY },
  { label: "7 days", ms: MILLISECONDS_IN_WEEK },
  { label: "4 weeks", ms: 4 * MILLISECONDS_IN_WEEK },
]

const RETENTION_BYTES_OPTIONS = [
  { label: "Not Set", value: -1 },
  { label: "1 GB", value: BYTES_IN_GB },
  { label: "10 GB", value: BYTES_IN_GB * 10 },
  { label: "20 GB", value: BYTES_IN_GB * 20 },
  { label: "50 GB", value: BYTES_IN_GB * 50 },
]

const name = ref("")
const partitions = ref(1)
const cleanupPolicy = ref("delete")
const minInSyncReplicas = ref<number | "">("")
const replicationFactor = ref<number | "">("")
const retentionMs = ref(MILLISECONDS_IN_WEEK)
const retentionBytes = ref(-1)
const maxMessageBytes = ref<number | "">("")
const customParams = ref<Array<{ name: string; value: string }>>([])
const submitting = ref(false)
const error = ref("")

const nameValid = computed(() => TOPIC_NAME_PATTERN.test(name.value.trim()))
const canSubmit = computed(() => nameValid.value && partitions.value >= 1 && !submitting.value)
const retentionHint = computed(() => formatDurationMs(Number(retentionMs.value) || 0))

const addCustomParam = () => {
  customParams.value.push({ name: "", value: "" })
}

const removeCustomParam = (index: number) => {
  customParams.value.splice(index, 1)
}

const submit = async () => {
  if (!canSubmit.value) return
  submitting.value = true
  error.value = ""
  try {
    await createTopic({
      name: name.value.trim(),
      partitions: Number(partitions.value) || 1,
      replication_factor: replicationFactor.value === "" ? undefined : Number(replicationFactor.value),
      cleanup_policy: cleanupPolicy.value,
      min_insync_replicas: minInSyncReplicas.value === "" ? undefined : Number(minInSyncReplicas.value),
      retention_ms: Number(retentionMs.value) || undefined,
      retention_bytes: retentionBytes.value,
      max_message_bytes: maxMessageBytes.value === "" ? undefined : Number(maxMessageBytes.value),
      custom_params: customParams.value.filter((item) => item.name.trim() && item.value.trim()),
    })
    toast.success(`Created topic ${name.value.trim()}`)
    await router.push(`/topics/${encodeURIComponent(name.value.trim())}`)
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to create topic")
    toast.error(error.value)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div class="page-container max-w-4xl">
    <nav class="text-sm text-muted-foreground">
      <NuxtLink to="/topics" class="hover:text-brand-500">Topics</NuxtLink>
      <span class="px-2">/</span>
      <span class="font-semibold text-foreground">Create</span>
    </nav>

    <AppPageHeader
      title="Create Topic"
      description="Set partitions, cleanup policy, retention, and optional custom Kafka parameters."
    />

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <AppCard>
      <form class="space-y-5" @submit.prevent="submit">
        <label class="block">
          <span class="auth-label">Topic Name <span class="text-red-500">*</span></span>
          <AppInput v-model="name" placeholder="Topic Name" />
          <p v-if="name && !nameValid" class="mt-1 text-xs text-red-500">
            Use letters, numbers, dots, underscores, and hyphens only.
          </p>
        </label>

        <div class="grid gap-4 md:grid-cols-2">
          <label>
            <span class="auth-label">Number of Partitions <span class="text-red-500">*</span></span>
            <AppInput v-model="partitions" type="number" min="1" />
          </label>
          <label>
            <span class="auth-label">Cleanup policy</span>
            <AppSelect v-model="cleanupPolicy">
              <option value="delete">Delete</option>
              <option value="compact">Compact</option>
              <option value="compact,delete">Compact,Delete</option>
            </AppSelect>
          </label>
        </div>

        <div class="grid gap-4 md:grid-cols-2">
          <label>
            <span class="auth-label">Min In Sync Replicas</span>
            <AppInput v-model="minInSyncReplicas" type="number" min="1" placeholder="Min In Sync Replicas" />
          </label>
          <label>
            <span class="auth-label">Replication Factor</span>
            <AppInput v-model="replicationFactor" type="number" min="1" placeholder="Replication Factor" />
          </label>
        </div>

        <div>
          <div class="mb-2 flex items-center justify-between">
            <span class="auth-label !mb-0">Time to retain data (in ms)</span>
            <span v-if="retentionHint" class="text-xs text-muted-foreground">{{ retentionHint }}</span>
          </div>
          <AppInput v-model="retentionMs" type="number" min="1" placeholder="Time to retain data (in ms)" />
          <div class="mt-2 flex flex-wrap gap-2">
            <button
              v-for="preset in RETENTION_PRESETS"
              :key="preset.label"
              type="button"
              class="rounded-lg border border-border px-3 py-1.5 text-xs font-medium"
              :class="Number(retentionMs) === preset.ms ? 'bg-surface-700 text-foreground' : 'bg-surface-800/60 text-muted-foreground hover:text-foreground'"
              @click="retentionMs = preset.ms"
            >
              {{ preset.label }}
            </button>
          </div>
        </div>

        <div class="grid gap-4 md:grid-cols-2">
          <label>
            <span class="auth-label">Max partition size in GB</span>
            <AppSelect :model-value="retentionBytes" @update:model-value="retentionBytes = Number($event)">
              <option v-for="option in RETENTION_BYTES_OPTIONS" :key="option.label" :value="option.value">
                {{ option.label }}
              </option>
            </AppSelect>
          </label>
          <label>
            <span class="auth-label">Maximum message size in bytes</span>
            <AppInput v-model="maxMessageBytes" type="number" min="1" placeholder="Maximum message size" />
          </label>
        </div>

        <div>
          <h2 class="section-title mb-3">Custom parameters</h2>
          <div v-for="(param, index) in customParams" :key="index" class="mb-2 grid gap-2 md:grid-cols-[1fr,1fr,auto]">
            <AppInput v-model="param.name" placeholder="Config name" />
            <AppInput v-model="param.value" placeholder="Config value" />
            <AppButton @click="removeCustomParam(index)">Remove</AppButton>
          </div>
          <AppButton @click="addCustomParam">+ Add Custom Parameter</AppButton>
        </div>

        <div class="flex gap-2 pt-2">
          <AppButton as="link" to="/topics">Cancel</AppButton>
          <AppButton variant="primary" :disabled="!canSubmit" @click="submit">
            {{ submitting ? "Creating..." : "Create topic" }}
          </AppButton>
        </div>
      </form>
    </AppCard>
  </div>
</template>
