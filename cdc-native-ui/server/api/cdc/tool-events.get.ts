import { orchestratorFetch } from "../../utils/orchestratorFetch"
import { buildQuery } from "../../utils/query"

// Copy & Sync timeline (ADR 0009): ?tool=&profile_id=&kind=&status=&since=&until=&limit=
export default defineEventHandler((event) =>
  orchestratorFetch(event, `/cdc/tool-events${buildQuery(getQuery(event))}`)
)
