import { orchestratorFetch } from "../../../../../utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const jobId = getRouterParam(event, "jobId")
  return orchestratorFetch(event, `/cdc/pgcopy/jobs/${encodeURIComponent(String(jobId))}/logs`)
})
