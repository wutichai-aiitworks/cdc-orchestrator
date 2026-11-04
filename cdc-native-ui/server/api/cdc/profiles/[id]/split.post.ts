import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

/** Move some tables of a pipeline into a new Draft pipeline (same source and target). */
export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  if (!id) throw createError({ statusCode: 400, statusMessage: "Profile ID is required" })
  const raw = (await readBody(event).catch(() => ({}))) || {}
  const body = {
    name: String(raw.name ?? "").trim(),
    description: typeof raw.description === "string" ? raw.description : undefined,
    tables: Array.isArray(raw.tables) ? raw.tables.map((t: unknown) => String(t)) : [],
  }
  return orchestratorFetch(event, `/cdc/profiles/${id}/split`, { method: "POST", body })
})
