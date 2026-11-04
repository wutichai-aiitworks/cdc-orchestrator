import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler((event) => orchestratorFetch(event, "/cdc/pgdump/jobs"))
