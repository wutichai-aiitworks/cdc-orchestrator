import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  if (!id) throw createError({ statusCode: 400, statusMessage: "Profile ID is required" })
  const raw = (await readBody(event).catch(() => ({}))) || {}
  const body = {
    clear_target: raw.clear_target === true,
    tables: Array.isArray(raw.tables) ? raw.tables.map((t: unknown) => String(t)) : [],
  }
  return orchestratorFetch(event, `/cdc/profiles/${id}/initial-load`, { method: "POST", body })
})
