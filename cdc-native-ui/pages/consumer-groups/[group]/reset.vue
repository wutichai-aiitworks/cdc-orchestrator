<script setup lang="ts">
import { formatApiError } from "~/utils/apiError"

const route = useRoute()
const router = useRouter()
const toast = useToast()
const { fetchGroup, resetGroup } = useKafkaAdmin()
const group = computed(() => decodeURIComponent(String(route.params.group || "")))
const resetType = ref("earliest")
const topic = ref("")
const timestampMs = ref("")
const submitting = ref(false)
const error = ref("")
const topics = ref<string[]>([])

onMounted(async () => {
  try {
    const detail = await fetchGroup(group.value)
    topics.value = [...new Set((detail.entries || []).map((row) => row.topic))]
    topic.value = topics.value[0] || ""
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load group")
  }
})

const submit = async () => {
  submitting.value = true
  error.value = ""
  try {
    await resetGroup(group.value, {
      reset_type: resetType.value,
      topic: topic.value || undefined,
      timestamp_ms: resetType.value === "timestamp" ? Number(timestampMs.value) : undefined,
    })
    toast.success("Offsets reset")
    await router.push(`/consumer-groups/${encodeURIComponent(group.value)}`)
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to reset offsets")
    toast.error(error.value)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div class="page-container max-w-3xl">
    <AppPageHeader :title="`Reset offsets · ${group}`" description="Reset to earliest, latest, or a timestamp.">
      <AppButton as="link" :to="`/consumer-groups/${encodeURIComponent(group)}`">Cancel</AppButton>
      <AppButton variant="primary" :disabled="submitting" @click="submit">Reset</AppButton>
    </AppPageHeader>
    <AppStateBlock v-if="error" kind="error" :message="error" />
    <AppCard class="space-y-4">
      <label>
        <span class="auth-label">Reset type</span>
        <AppSelect v-model="resetType">
          <option value="earliest">EARLIEST</option>
          <option value="latest">LATEST</option>
          <option value="timestamp">TIMESTAMP</option>
        </AppSelect>
      </label>
      <label>
        <span class="auth-label">Topic (optional)</span>
        <AppSelect v-model="topic">
          <option value="">All assigned topics</option>
          <option v-for="name in topics" :key="name" :value="name">{{ name }}</option>
        </AppSelect>
      </label>
      <label v-if="resetType === 'timestamp'">
        <span class="auth-label">Timestamp (epoch ms)</span>
        <AppInput v-model="timestampMs" type="number" placeholder="1710000000000" />
      </label>
    </AppCard>
  </div>
</template>
