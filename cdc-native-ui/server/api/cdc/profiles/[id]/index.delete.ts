import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

// Deletes the pipeline, its connectors and (log-based Postgres) its replication slot unless
// `keep_slot=true` (B-02). Returns {connectors_deleted, slot}.
export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  if (!id) throw createError({ statusCode: 400, statusMessage: "Profile ID is required" })
  const keep = String(getQuery(event).keep_slot || "") === "true"
  return orchestratorFetch(event, `/cdc/profiles/${id}${keep ? "?keep_slot=true" : ""}`, { method: "DELETE" })
})
