/** Plain-language names for pipeline history events (`profile.history[].event`). */
export const EVENT_LABELS: Record<string, string> = {
  profile_created: "Created",
  profile_updated: "Configuration updated",
  profile_started: "Started",
  profile_autostarted: "Started after orchestrator restart",
  profile_stopped: "Stopped",
  profile_paused: "Paused",
  profile_resumed: "Resumed",
  profile_restarted: "Restarted",
  profile_resynced: "Resynced",
  profile_initial_load: "Initial load finished",
  profile_initial_load_failed: "Initial load failed",
  profile_initial_load_cancelled: "Initial load cancelled",
  profile_enable_changed: "Enabled state changed",
  profile_test_initial_sync: "Test initial sync passed",
  profile_test_initial_sync_failed: "Test initial sync failed",
  source_offsets_reset: "Source position reset — full snapshot before streaming",
}

const humanize = (raw: string) =>
  raw
    .replace(/^profile[._]/, "")
    .replace(/[._]/g, " ")
    .replace(/^\w/, (c) => c.toUpperCase())

export const eventLabel = (event: string) => EVENT_LABELS[event] ?? humanize(event)

/** Audit actions (`profile.start`, `connector.failed`, …) as words. */
export const auditActionLabel = (action: string) => humanize(action)
