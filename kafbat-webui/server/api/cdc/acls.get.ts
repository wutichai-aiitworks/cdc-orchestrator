import { buildQuery } from "../../utils/query"
import { orchestratorFetch } from "../../utils/orchestratorFetch"

export default defineEventHandler((event) =>
  orchestratorFetch(event, `/cdc/acls${buildQuery(getQuery(event))}`)
)
