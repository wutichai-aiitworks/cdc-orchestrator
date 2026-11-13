/**
 * Notification channels and rule conditions for the Alerts page (ADR 0015).
 * Pure: metadata, form defaults and form → API payload. API calls live in useOperationsM3.
 */

export type ChannelType = "webhook" | "email" | "line" | "slack" | "discord" | "teams"

export const REDACTED = "********"

export type ChannelSettings = {
  url?: string
  legacySecretHeader?: boolean
  smtpHost?: string
  smtpPort?: number | null
  smtpSecurity?: "starttls" | "tls" | "none"
  smtpUsername?: string
  from?: string
  to?: string[]
  lineTo?: string
}

export type NotificationChannel = {
  id: string
  name: string
  type: ChannelType
  enabled: boolean
  settings: ChannelSettings
  /** Stored secrets come back as "********"; send that back to keep them. */
  secrets: Record<string, string>
  createdAt?: string
  updatedAt?: string
  migratedFromRule?: string | null
  /** Secret-free destination label (list responses only). */
  target?: string
}

export type NotificationDelivery = {
  delivery_id: string
  timestamp: string
  channel_id?: string | null
  channel_name: string
  channel_type: string
  rule_id?: string | null
  rule_name?: string | null
  condition: string
  subject: string
  outcome: "delivered" | "failed" | "dropped"
  attempts: number
  status_code?: number | null
  error?: string | null
  duration_ms: number
  test: boolean
}

export type SecretField = { key: string; label: string; required: boolean; placeholder: string; help?: string }

export type ChannelTypeMeta = {
  type: ChannelType
  label: string
  description: string
  secrets: SecretField[]
  /** Where to get the credentials. */
  setupHelp: string
}

export const CHANNEL_TYPES: ChannelTypeMeta[] = [
  {
    type: "email",
    label: "Email (SMTP)",
    description: "Any SMTP server: Gmail, Microsoft 365, SES, Postfix.",
    secrets: [{ key: "smtpPassword", label: "SMTP password", required: false, placeholder: "App password or SMTP credential" }],
    setupHelp: "Gmail and Microsoft 365 need an app password (or SMTP AUTH enabled) for the account.",
  },
  {
    type: "line",
    label: "LINE",
    description: "Push messages through a LINE Official Account (Messaging API).",
    secrets: [
      {
        key: "channelAccessToken",
        label: "Channel access token",
        required: true,
        placeholder: "Long-lived channel access token",
        help: "LINE Developers console → your Messaging API channel → Messaging API tab.",
      },
    ],
    setupHelp:
      "LINE Notify was discontinued in March 2025. Create a Messaging API channel, add the bot as a friend (or to a group), and use the user or group id it receives in webhook events.",
  },
  {
    type: "slack",
    label: "Slack",
    description: "Post to a channel through a Slack incoming webhook.",
    secrets: [{ key: "webhookUrl", label: "Incoming webhook URL", required: true, placeholder: "https://hooks.slack.com/services/…" }],
    setupHelp: "Slack app → Incoming Webhooks → Add New Webhook to Workspace.",
  },
  {
    type: "teams",
    label: "Microsoft Teams",
    description: "Post an Adaptive Card through a Teams workflow.",
    secrets: [{ key: "webhookUrl", label: "Workflow URL", required: true, placeholder: "https://…logic.azure.com/… or …powerplatform.com/…" }],
    setupHelp: "In the channel: Workflows → \"Post to a channel when a webhook request is received\". Office 365 connector webhooks are retired.",
  },
  {
    type: "discord",
    label: "Discord",
    description: "Post an embed to a Discord channel.",
    secrets: [{ key: "webhookUrl", label: "Webhook URL", required: true, placeholder: "https://discord.com/api/webhooks/…" }],
    setupHelp: "Channel settings → Integrations → Webhooks → New Webhook → Copy URL.",
  },
  {
    type: "webhook",
    label: "Webhook",
    description: "POST JSON to your own endpoint (PagerDuty, Opsgenie, n8n, custom).",
    secrets: [
      {
        key: "signingSecret",
        label: "Signing secret",
        required: false,
        placeholder: "Optional — used to sign each request",
        help: "Requests carry X-CDC-Signature: t=<unix>,v1=<hex HMAC-SHA256(secret, \"t.body\")>.",
      },
    ],
    setupHelp: "The body has event, connector, from, to, ts, errors, plus severity, title, summary, subject and rule.",
  },
]

export const channelMeta = (type: ChannelType): ChannelTypeMeta =>
  CHANNEL_TYPES.find((t) => t.type === type) ?? CHANNEL_TYPES[CHANNEL_TYPES.length - 1]

export const emptyChannel = (type: ChannelType = "email"): NotificationChannel => ({
  id: "",
  name: "",
  type,
  enabled: true,
  settings: type === "email" ? { smtpSecurity: "starttls", smtpPort: null, to: [] } : {},
  secrets: {},
})

/** "a@x.com, b@y.com\nc@z.com" → ["a@x.com", "b@y.com", "c@z.com"] */
export const splitRecipients = (text: string): string[] =>
  text
    .split(/[,;\n]/)
    .map((s) => s.trim())
    .filter(Boolean)

/**
 * Form → API body: only the settings and secrets this type uses. Secret inputs left blank on
 * a saved channel become "********" (keep); a blank on a new channel is omitted.
 */
export const channelPayload = (
  form: NotificationChannel,
  opts: { recipients?: string; storedSecrets?: string[]; clearSecrets?: string[] } = {},
): NotificationChannel => {
  const s = form.settings
  const settings: ChannelSettings = {}
  switch (form.type) {
    case "webhook":
      settings.url = s.url?.trim()
      if (s.legacySecretHeader) settings.legacySecretHeader = true
      break
    case "email":
      settings.smtpHost = s.smtpHost?.trim()
      settings.smtpPort = s.smtpPort ? Number(s.smtpPort) : null
      settings.smtpSecurity = s.smtpSecurity || "starttls"
      settings.smtpUsername = s.smtpUsername?.trim() || undefined
      settings.from = s.from?.trim()
      settings.to = opts.recipients !== undefined ? splitRecipients(opts.recipients) : s.to ?? []
      break
    case "line":
      settings.lineTo = s.lineTo?.trim()
      break
  }
  const secrets: Record<string, string> = {}
  for (const field of channelMeta(form.type).secrets) {
    const typed = (form.secrets[field.key] ?? "").trim()
    if (opts.clearSecrets?.includes(field.key)) secrets[field.key] = ""
    else if (typed && typed !== REDACTED) secrets[field.key] = typed
    else if (opts.storedSecrets?.includes(field.key)) secrets[field.key] = REDACTED
  }
  return { id: form.id, name: form.name.trim(), type: form.type, enabled: form.enabled, settings, secrets }
}

export const SMTP_DEFAULT_PORT = { starttls: 587, tls: 465, none: 25 } as const

// ── Rule conditions ─────────────────────────────────────────────────────────

export type ConditionMeta = {
  value: string
  label: string
  /** What the glob filter is matched against. */
  subject: "connector" | "topic" | "pipeline"
  hint: string
  needsMinutes?: boolean
  /** latency_above: seconds threshold + sustained minutes. */
  needsThreshold?: boolean
}

export const CONDITIONS: ConditionMeta[] = [
  { value: "connector_failed", label: "Connector failed", subject: "connector", hint: "A connector or one of its tasks enters FAILED." },
  {
    value: "connector_failed_for",
    label: "Connector failed for …",
    subject: "connector",
    hint: "Still FAILED after the given minutes. Fires once per failure — a reminder when nobody fixed it.",
    needsMinutes: true,
  },
  { value: "connector_recovered", label: "Connector recovered", subject: "connector", hint: "Back to RUNNING from FAILED, PAUSED or UNASSIGNED." },
  { value: "connector_paused", label: "Connector paused", subject: "connector", hint: "A connector enters PAUSED." },
  { value: "connector_state_changed", label: "Any connector state change", subject: "connector", hint: "Every state change. Noisy — pair with a cooldown." },
  { value: "pipeline_failed", label: "Pipeline operation failed", subject: "pipeline", hint: "Start, resync, initial load or another pipeline action failed." },
  { value: "dlq_growth", label: "Dead-letter queue growing", subject: "topic", hint: "New records arrived in a pipeline's DLQ topic." },
  {
    value: "latency_above",
    label: "Pipeline latency above …",
    subject: "pipeline",
    hint: "End-to-end latency stays above the threshold for the given minutes. Sends a recovery message when it drops back.",
    needsThreshold: true,
  },
]

export const conditionMeta = (value: string): ConditionMeta =>
  CONDITIONS.find((c) => c.value === (value === "connector_paused_N_min" ? "connector_paused" : value)) ?? {
    value,
    label: value,
    subject: "connector",
    hint: "",
  }

export const FILTER_PLACEHOLDER: Record<ConditionMeta["subject"], string> = {
  connector: "* or orders-*",
  topic: "* or dlq.orders-*",
  pipeline: "* or prod-*",
}
