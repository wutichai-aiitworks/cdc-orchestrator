import { orchestratorFetch } from "../../utils/orchestratorFetch"

/** Interval and next run per pipeline (orchestrator GET /cdc/schedule). */
export default defineEventHandler((event) => orchestratorFetch(event, "/cdc/schedule"))
