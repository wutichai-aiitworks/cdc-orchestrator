import { orchestratorFetch } from "../../../utils/orchestratorFetch"
import { buildQuery } from "../../../utils/query"

export default defineEventHandler((event) =>
  orchestratorFetch(event, `/cdc/audit/events${buildQuery(getQuery(event))}`)
)
