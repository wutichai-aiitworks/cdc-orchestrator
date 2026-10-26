import { orchestratorFetch } from "~/server/utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const jobId = getRouterParam(event, "jobId")
  return orchestratorFetch(event, `/cdc/redisshake/jobs/${encodeURIComponent(String(jobId))}`)
})
