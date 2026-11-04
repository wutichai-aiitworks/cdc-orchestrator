import { orchestratorFetch } from "../../../../../utils/orchestratorFetch"

// ?dry_run=true runs a preview (itemized changes, nothing written).
export default defineEventHandler((event) => {
  const id = getRouterParam(event, "id")
  const dry = getQuery(event).dry_run === "true" ? "?dry_run=true" : ""
  return orchestratorFetch(event, `/cdc/rsync/profiles/${encodeURIComponent(String(id))}/run${dry}`, { method: "POST" })
})
