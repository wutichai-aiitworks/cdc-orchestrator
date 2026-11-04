import type { ConnCheck } from "~/utils/transferTools/types"

type SidecarTest = { ok: boolean; target?: string; facts?: string[]; warning?: string; error?: string; hint?: string | null; [k: string]: unknown }

/**
 * "Test connection" for a Copy & Sync tool: POST /cdc/{tool}/test-connection connects from the
 * tool's sidecar, the same way a run does. A saved password (********) is filled in by the API
 * for the saved profile's own login only.
 */
export const transferTester =
  (tool: "pgcopy" | "pgloader" | "mongocopy" | "redisshake") =>
  (side: "source" | "target", options: () => Record<string, unknown>, profileId: () => string | undefined) =>
  async (connection: Record<string, unknown>): Promise<Omit<ConnCheck, "at">> => {
    const r = await $fetch<SidecarTest>(`/api/cdc/${tool}/test-connection`, {
      method: "POST",
      body: { connection, side, options: options(), profile_id: profileId() },
    })
    return r.ok ? { ok: true, facts: r.facts || [], warning: r.warning, raw: r } : { ok: false, error: r.error, hint: r.hint }
  }
