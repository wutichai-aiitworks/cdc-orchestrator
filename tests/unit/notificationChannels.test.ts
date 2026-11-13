// Alerts page channel form → API payload (cdc-native-ui/utils/notificationChannels.ts), ADR 0015.
import test from "node:test"
import assert from "node:assert/strict"

import {
  REDACTED,
  channelPayload,
  conditionMeta,
  emptyChannel,
  splitRecipients,
} from "../../cdc-native-ui/utils/notificationChannels.ts"

test("only the chosen type's settings and secrets are sent", () => {
  const form = { ...emptyChannel("line"), name: " Ops LINE ", settings: { lineTo: " Uabc ", url: "https://x", smtpHost: "h" }, secrets: { channelAccessToken: "tok", webhookUrl: "leftover" } }
  const body = channelPayload(form)
  assert.equal(body.name, "Ops LINE")
  assert.deepEqual(body.settings, { lineTo: "Uabc" })
  assert.deepEqual(body.secrets, { channelAccessToken: "tok" })
})

test("a blank secret keeps the stored one, a cleared one is sent empty, a new one replaces it", () => {
  const form = { ...emptyChannel("email"), id: "c1", name: "Mail", settings: { smtpHost: "smtp.x", from: "a@x.com" }, secrets: { smtpPassword: "" } }
  assert.deepEqual(channelPayload(form, { storedSecrets: ["smtpPassword"] }).secrets, { smtpPassword: REDACTED })
  assert.deepEqual(channelPayload(form, { storedSecrets: ["smtpPassword"], clearSecrets: ["smtpPassword"] }).secrets, { smtpPassword: "" })
  assert.deepEqual(channelPayload({ ...form, secrets: { smtpPassword: "new" } }, { storedSecrets: ["smtpPassword"] }).secrets, { smtpPassword: "new" })
  assert.deepEqual(channelPayload(form).secrets, {}, "new channel with a blank secret sends nothing")
})

test("email recipients come from the text box; port blank means the default for the security mode", () => {
  const form = { ...emptyChannel("email"), name: "Mail", settings: { smtpHost: "smtp.x", from: "a@x.com", smtpSecurity: "tls" as const, smtpPort: null } }
  const body = channelPayload(form, { recipients: "ops@x.com; dba@x.com\n\n oncall@x.com " })
  assert.deepEqual(body.settings.to, ["ops@x.com", "dba@x.com", "oncall@x.com"])
  assert.equal(body.settings.smtpPort, null)
  assert.equal(body.settings.smtpSecurity, "tls")
  assert.deepEqual(splitRecipients(""), [])
})

test("conditions: the legacy paused spelling maps to connector_paused; unknown ones still render", () => {
  assert.equal(conditionMeta("connector_paused_N_min").value, "connector_paused")
  assert.equal(conditionMeta("dlq_growth").subject, "topic")
  assert.equal(conditionMeta("something_new").label, "something_new")
})
