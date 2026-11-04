import { orchestratorFetch } from "../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) =>
  orchestratorFetch(event, "/cdc/mongocopy/validate", { method: "POST", body: await readBody(event) }),
)
