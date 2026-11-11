import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

/** Full copy of a pipeline as a new Draft under a new name (original untouched). */
export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  if (!id) throw createError({ statusCode: 400, statusMessage: "Profile ID is required" })
  const raw = (await readBody(event).catch(() => ({}))) || {}
  const body = {
    name: String(raw.name ?? "").trim(),
    description: typeof raw.description === "string" ? raw.description : undefined,
  }
  return orchestratorFetch(event, `/cdc/profiles/${id}/duplicate`, { method: "POST", body })
})
