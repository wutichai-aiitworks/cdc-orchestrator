import { orchestratorFetch } from "../../../../../utils/orchestratorFetch"

// Ad-hoc SFTP send of a finished job's dump file. Secrets pass through; nothing is stored here.
export default defineEventHandler(async (event) => {
  const jobId = getRouterParam(event, "jobId")
  return orchestratorFetch(event, `/cdc/pgdump/jobs/${encodeURIComponent(String(jobId))}/send`, {
    method: "POST",
    body: await readBody(event),
  })
})
