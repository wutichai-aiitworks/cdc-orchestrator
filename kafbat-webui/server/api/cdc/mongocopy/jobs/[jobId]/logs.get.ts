import { orchestratorFetch } from "../../../../../utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const jobId = getRouterParam(event, "jobId")
  return orchestratorFetch(event, `/cdc/mongocopy/jobs/${encodeURIComponent(String(jobId))}/logs`)
})
