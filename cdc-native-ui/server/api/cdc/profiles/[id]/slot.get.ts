import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

// The replication slot deleting this pipeline would drop (exists, active, WAL held). Read-only.
export default defineEventHandler((event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  if (!id) throw createError({ statusCode: 400, statusMessage: "Profile ID is required" })
  return orchestratorFetch(event, `/cdc/profiles/${id}/slot`)
})
