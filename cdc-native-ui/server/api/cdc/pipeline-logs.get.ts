import { orchestratorFetch } from "../../utils/orchestratorFetch"
import { buildQuery } from "../../utils/query"

export default defineEventHandler((event) =>
  orchestratorFetch(event, `/cdc/pipeline-logs${buildQuery(getQuery(event))}`)
)
