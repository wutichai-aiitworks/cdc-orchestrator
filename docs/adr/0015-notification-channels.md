# ADR 0015 — Notification channels (webhook, email, LINE, Slack, Discord, Teams)

Status: Accepted (built; see "Verified" / "Not verified")
Owner: Wutichai
Last updated: 2026-11-12

## Context

Alert rules (`cdc_config` kind `rule`) could only call a webhook. Each rule carried its own
`webhookUrl` and `webhookSecret`, and:

- the secret was stored in plain text and returned by `GET /cdc/notifications/rules`;
- editing a rule in the UI erased the secret (the form sent `""`, the PUT replaced the rule);
- the secret went out as a plain `X-Webhook-Secret` header, not a signature;
- delivery ran inline in the connector watcher's poll loop (15 s timeout, no retry), so one slow
  endpoint stalled connector watching for every pipeline, and failures only reached the log;
- only "connector failed" and "connector paused" existed; a flapping connector alerted on
  every poll.

Operators asked for email (SMTP), LINE and the chat tools they already use.

## Decision

1. **Channels are their own records** (`cdc_config` kind `channel`), reused by many rules.
   A rule lists `channelIds`. One place to rotate a token; one "Send test" per destination.
   Types: `webhook`, `email`, `line`, `slack`, `discord`, `teams`.
   - **LINE uses the Messaging API** (push message, channel access token, `to` = user / group /
     room id). LINE Notify was shut down on 2025-03-31 and is not an option.
   - **Teams uses a Workflows ("When a Teams webhook request is received") URL** with an
     Adaptive Card. Office 365 connector webhooks are retired.
   - Slack / Discord / Teams webhook URLs *are* credentials and are stored as secrets.
2. **Secrets are encrypted and write-only.** AES-256-GCM with a key from `ORCH_SECRET_KEY`
   (32 bytes, base64 or hex), stored as `v1:<base64 nonce‖ciphertext>`. The API returns
   `********` for a stored secret; sending `********` (or omitting the field) keeps it, `""`
   clears it. As in `saved_secret.rs`, a kept secret may not follow a changed destination:
   changing an email host / username, or the URL of a webhook that sends the secret in a
   header, requires entering the secret again. Without `ORCH_SECRET_KEY`, channels that need
   a secret cannot be saved (400 explains how to set it); `/capability` reports
   `features.notification_secrets`.
3. **Webhooks are signed.** `X-CDC-Signature: t=<unix>,v1=<hex HMAC-SHA256(secret, "t.body")>`
   (Stripe-style, so receivers can reject replays). Channels migrated from old rules keep the
   legacy `X-Webhook-Secret` header (`legacySecretHeader: true`) so existing receivers work.
   The JSON body keeps the old fields (`event, connector, from, to, ts, errors`) and adds
   `severity, title, summary, subject, rule`.
4. **Delivery is queued.** Matching rules enqueue jobs (bounded queue, 1000); a worker sends
   up to 8 at a time with 4 attempts (backoff 2 s, 10 s, 30 s). 4xx other than 408/429 is not
   retried. Every final result is written to `cdc_notification_deliveries` (rqlite; trimmed with
   `ORCH_AUDIT_RQLITE_MAX_ROWS`) or an in-memory ring of 500 without rqlite. A full queue drops
   the job and records `dropped`. The watcher never waits on a channel.
5. **Conditions:** `connector_failed`, `connector_paused`, `connector_recovered` (back to
   RUNNING from FAILED / PAUSED / UNASSIGNED), `connector_failed_for` (still FAILED after
   `conditionValue.minutes`, once per failure), `connector_state_changed`, `dlq_growth`
   (filter matches the DLQ topic), `pipeline_failed` (an audit event with outcome `failure`
   for a pipeline, other than connector state changes; filter matches the pipeline name).
   The old `connector_paused_N_min` value is read as `connector_paused`.
6. **Cooldown per rule and subject** (`cooldownMinutes`, default 0 for existing rules, 5 in the
   UI for new ones), in memory on the instance that runs the watcher. `connector_recovered`
   is never suppressed by a failure's cooldown (separate key).
7. **Migration on startup:** a rule with `webhookUrl` and no `channelIds` gets a webhook
   channel (id = UUIDv5 of the rule id, so a rerun is a no-op) and its legacy fields are
   cleared. If the rule has a secret and `ORCH_SECRET_KEY` is unset, the rule is left as it
   was and still delivers through the legacy fields (warning logged) until a key is set.
8. **Test before save:** `POST /cdc/notifications/channels/test` sends one sample message
   (no retry) from a draft, using stored secrets when the draft has an `id` and `********`.

## API

| Method | Path | |
|---|---|---|
| GET, POST | `/cdc/notifications/channels` | list (secrets redacted) / create |
| GET, PUT, DELETE | `/cdc/notifications/channels/{id}` | DELETE refuses while rules use it |
| POST | `/cdc/notifications/channels/test` | send a sample from a draft |
| POST | `/cdc/notifications/channels/{id}/test` | send a sample from a saved channel |
| GET | `/cdc/notifications/deliveries` | `channel_id`, `rule_id`, `outcome`, `limit` |

Rules gain `channelIds`, `cooldownMinutes`; `webhookUrl` / `webhookSecret` remain readable
for old clients (secret redacted) and are cleared by the migration.

## Consequences

- Rotating a token is one edit; every rule using the channel picks it up.
- Losing `ORCH_SECRET_KEY` makes stored channel secrets unreadable; channels report
  "secret cannot be decrypted" on test and in deliveries until the secret is entered again.
- Cooldown and `connector_failed_for` timers are per instance and reset on restart (at worst
  one repeated alert after a restart).

## Not in this change

- Telegram, PagerDuty, Opsgenie (each is a new `type` + renderer).
- ~~MCP write tools for channels~~ — added in ADR 0017 (`cdc_notification_channel_upsert` etc.; output stays redacted).
- Per-channel quiet hours and digesting.
