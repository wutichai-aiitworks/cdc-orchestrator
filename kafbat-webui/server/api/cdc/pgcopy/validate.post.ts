import { orchestratorFetch } from "../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) =>
  orchestratorFetch(event, "/cdc/pgcopy/validate", { method: "POST", body: await readBody(event) }),
)
