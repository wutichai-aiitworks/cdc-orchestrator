<script setup lang="ts">
/**
 * Create or edit a notification channel (ADR 0015). Secrets are write-only: a saved secret
 * shows as "Saved" and is kept unless the operator types a new one or clears it.
 * "Send test" sends a sample from the form as it is, before saving.
 */
import { CheckCircle2, CircleX, Info, Send } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import {
  CHANNEL_TYPES,
  REDACTED,
  SMTP_DEFAULT_PORT,
  channelMeta,
  channelPayload,
  emptyChannel,
  type ChannelType,
  type NotificationChannel,
  type NotificationDelivery,
} from "~/utils/notificationChannels"

const props = defineProps<{
  /** Channel to edit; null for a new one. */
  channel: NotificationChannel | null
  secretsAvailable: boolean
}>()
const emit = defineEmits<{ close: []; saved: [NotificationChannel] }>()

const { createChannel, updateChannel, testDraftChannel } = useOperationsM3()
const toast = useToast()

const isEdit = computed(() => !!props.channel?.id)
const form = reactive<NotificationChannel>(
  props.channel
    ? JSON.parse(JSON.stringify({ ...props.channel, secrets: {} }))
    : emptyChannel("email"),
)
const recipients = ref((props.channel?.settings.to ?? []).join(", "))
/** Secret fields stored on the server for this channel. */
const stored = computed(() =>
  Object.entries(props.channel?.secrets ?? {})
    .filter(([, v]) => v === REDACTED)
    .map(([k]) => k),
)
const clearing = ref<string[]>([])
const saving = ref(false)
const testing = ref(false)
const error = ref("")
const testResult = ref<NotificationDelivery | null>(null)

const meta = computed(() => channelMeta(form.type))
const needsKey = computed(
  () => !props.secretsAvailable && meta.value.secrets.some((f) => f.required || (form.secrets[f.key] ?? "").trim()),
)

const pickType = (type: ChannelType) => {
  if (isEdit.value || form.type === type) return
  const fresh = emptyChannel(type)
  form.type = type
  form.settings = fresh.settings
  form.secrets = {}
  testResult.value = null
  error.value = ""
}

const payload = () =>
  channelPayload(form, {
    recipients: recipients.value,
    storedSecrets: stored.value,
    clearSecrets: clearing.value,
  })

const toggleClear = (key: string) => {
  clearing.value = clearing.value.includes(key) ? clearing.value.filter((k) => k !== key) : [...clearing.value, key]
  form.secrets[key] = ""
}

const sendTest = async () => {
  testing.value = true
  error.value = ""
  testResult.value = null
  try {
    testResult.value = await testDraftChannel({ ...payload(), name: form.name || "Draft channel" })
  } catch (e: any) {
    error.value = formatApiError(e, "Test could not be sent")
  } finally {
    testing.value = false
  }
}

const save = async () => {
  if (!form.name.trim()) {
    error.value = "Name is required."
    return
  }
  saving.value = true
  error.value = ""
  try {
    const body = payload()
    const saved = isEdit.value ? await updateChannel(body) : await createChannel(body)
    toast.success(isEdit.value ? "Channel updated." : "Channel created.")
    emit("saved", saved)
  } catch (e: any) {
    error.value = formatApiError(e, "Channel could not be saved")
  } finally {
    saving.value = false
  }
}

const portPlaceholder = computed(() => String(SMTP_DEFAULT_PORT[form.settings.smtpSecurity ?? "starttls"]))
</script>

<template>
  <Teleport to="body">
    <div class="modal-overlay" @click.self="emit('close')">
      <section class="auth-card flex max-h-[90vh] w-full max-w-2xl flex-col bg-card p-0" role="dialog" aria-modal="true">
        <header class="border-b border-border px-5 py-4">
          <h2 class="section-title mb-0">{{ isEdit ? "Edit channel" : "New notification channel" }}</h2>
          <p class="mt-1 text-sm text-muted-foreground">Where alert rules send their messages.</p>
        </header>

        <div class="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4">
          <!-- Type -->
          <div>
            <span class="auth-label">Type</span>
            <div class="grid grid-cols-2 gap-2 sm:grid-cols-3" role="radiogroup" aria-label="Channel type">
              <button
                v-for="t in CHANNEL_TYPES"
                :key="t.type"
                type="button"
                role="radio"
                :aria-checked="form.type === t.type"
                :disabled="isEdit && form.type !== t.type"
                class="rounded-lg border px-3 py-2 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-40"
                :class="form.type === t.type ? 'border-primary bg-primary/10' : 'border-border hover:bg-muted'"
                @click="pickType(t.type)"
              >
                <span class="flex items-center gap-2">
                  <ChannelBrandIcon :type="t.type" size="sm" />
                  <span class="text-sm font-medium text-foreground">{{ t.label }}</span>
                </span>
                <p class="mt-1 line-clamp-2 text-xs text-muted-foreground">{{ t.description }}</p>
              </button>
            </div>
          </div>

          <div class="flex items-start gap-2 rounded-lg border border-blue-500/25 bg-blue-500/5 px-3 py-2 text-xs text-blue-800 dark:text-blue-300">
            <Info class="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>{{ meta.setupHelp }}</span>
          </div>

          <label class="block">
            <span class="auth-label">Name</span>
            <AppInput v-model="form.name" :placeholder="`${meta.label} — operations`" />
          </label>

          <!-- Webhook -->
          <template v-if="form.type === 'webhook'">
            <label class="block">
              <span class="auth-label">URL</span>
              <AppInput v-model="form.settings.url" type="url" placeholder="https://alerts.example.com/cdc" />
            </label>
            <label class="flex items-center gap-2 text-sm text-foreground">
              <input v-model="form.settings.legacySecretHeader" type="checkbox" class="h-4 w-4 accent-brand-500" />
              Send the secret as <code class="font-mono text-xs">X-Webhook-Secret</code> (older receivers) instead of signing
            </label>
          </template>

          <!-- Email -->
          <template v-if="form.type === 'email'">
            <div class="grid grid-cols-1 gap-4 sm:grid-cols-[1fr_9rem_7rem]">
              <label class="block">
                <span class="auth-label">SMTP host</span>
                <AppInput v-model="form.settings.smtpHost" placeholder="smtp.gmail.com" />
              </label>
              <label class="block">
                <span class="auth-label">Security</span>
                <AppSelect :model-value="form.settings.smtpSecurity ?? 'starttls'" aria-label="SMTP security" @update:model-value="(v: string | number) => (form.settings.smtpSecurity = String(v) as 'starttls' | 'tls' | 'none')">
                  <option value="starttls">STARTTLS</option>
                  <option value="tls">TLS (implicit)</option>
                  <option value="none">None</option>
                </AppSelect>
              </label>
              <label class="block">
                <span class="auth-label">Port</span>
                <input v-model.number="form.settings.smtpPort" type="number" min="1" max="65535" class="auth-input" :placeholder="portPlaceholder" />
              </label>
            </div>
            <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
              <label class="block">
                <span class="auth-label">Username (optional)</span>
                <AppInput v-model="form.settings.smtpUsername" placeholder="alerts@example.com" />
              </label>
              <label class="block">
                <span class="auth-label">From</span>
                <AppInput v-model="form.settings.from" placeholder="CDC Alerts <alerts@example.com>" />
              </label>
            </div>
            <label class="block">
              <span class="auth-label">To (comma or one per line)</span>
              <textarea v-model="recipients" rows="2" class="auth-input !min-h-[4rem]" placeholder="ops@example.com, dba@example.com" />
            </label>
          </template>

          <!-- LINE -->
          <label v-if="form.type === 'line'" class="block">
            <span class="auth-label">Send to (user, group or room id)</span>
            <AppInput v-model="form.settings.lineTo" input-class="font-mono" placeholder="U4af4980629… or C1234…" />
          </label>

          <!-- Secrets -->
          <div v-for="field in meta.secrets" :key="field.key" class="block">
            <div class="mb-1.5 flex items-center justify-between gap-2">
              <span class="text-xs font-medium text-muted-foreground">
                {{ field.label }}<span v-if="!field.required"> (optional)</span>
              </span>
              <span v-if="stored.includes(field.key)" class="flex items-center gap-2 text-xs">
                <span :class="clearing.includes(field.key) ? 'text-amber-600 dark:text-amber-400' : 'text-emerald-600 dark:text-emerald-400'">
                  {{ clearing.includes(field.key) ? "Will be removed" : "Saved" }}
                </span>
                <button v-if="!field.required" type="button" class="text-muted-foreground underline hover:text-foreground" @click="toggleClear(field.key)">
                  {{ clearing.includes(field.key) ? "Keep" : "Remove" }}
                </button>
              </span>
            </div>
            <AppInput
              :model-value="form.secrets[field.key] ?? ''"
              type="password"
              :disabled="clearing.includes(field.key)"
              :placeholder="stored.includes(field.key) ? 'Saved — leave blank to keep' : field.placeholder"
              :aria-label="field.label"
              @update:model-value="(v: string | number) => (form.secrets[field.key] = String(v))"
            />
            <p v-if="field.help" class="mt-1 text-xs text-muted-foreground">{{ field.help }}</p>
          </div>

          <div v-if="needsKey" class="alert-warning !items-start text-xs">
            Secrets can't be stored until <code class="font-mono">ORCH_SECRET_KEY</code> is set on the orchestrator
            (<code class="font-mono">openssl rand -base64 32</code>, then restart).
          </div>

          <label class="flex items-center gap-2 text-sm text-foreground">
            <input v-model="form.enabled" type="checkbox" class="h-4 w-4 accent-brand-500" />
            Enabled
          </label>

          <div
            v-if="testResult"
            class="flex items-start gap-2 rounded-lg border px-3 py-2 text-sm"
            :class="testResult.outcome === 'delivered'
              ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-800 dark:text-emerald-300'
              : 'border-red-500/30 bg-red-500/10 text-red-800 dark:text-red-300'"
            role="status"
          >
            <component :is="testResult.outcome === 'delivered' ? CheckCircle2 : CircleX" class="mt-0.5 h-4 w-4 shrink-0" />
            <span class="break-words">
              <template v-if="testResult.outcome === 'delivered'">
                Test delivered in {{ testResult.duration_ms }} ms<template v-if="testResult.status_code"> (HTTP {{ testResult.status_code }})</template>.
              </template>
              <template v-else>Test failed: {{ testResult.error || "unknown error" }}</template>
            </span>
          </div>
          <AppStateBlock v-if="error" kind="error" :message="error" />
        </div>

        <footer class="flex items-center justify-between gap-2 border-t border-border px-5 py-3">
          <AppButton :disabled="testing || saving" @click="sendTest">
            <Send class="h-4 w-4" :stroke-width="1.75" />
            {{ testing ? "Sending…" : "Send test" }}
          </AppButton>
          <div class="flex gap-2">
            <AppButton :disabled="saving" @click="emit('close')">Cancel</AppButton>
            <AppButton variant="primary" :disabled="saving || !form.name.trim()" @click="save">
              {{ saving ? "Saving…" : isEdit ? "Save changes" : "Create channel" }}
            </AppButton>
          </div>
        </footer>
      </section>
    </div>
  </Teleport>
</template>
