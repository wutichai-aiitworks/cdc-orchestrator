import { buildQuery } from "../../utils/query"
import { orchestratorFetch } from "../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const q = getQuery(event)
  return orchestratorFetch(event, `/cdc/consumer-groups${buildQuery(q)}`)
})
