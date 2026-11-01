import { orchestratorFetch } from "../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const body = await readBody(event)
  return orchestratorFetch(event, "/cdc/ksqldb/drop", { method: "POST", body })
})
