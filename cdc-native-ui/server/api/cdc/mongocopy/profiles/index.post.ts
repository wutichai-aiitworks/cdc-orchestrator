import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) =>
  orchestratorFetch(event, "/cdc/mongocopy/profiles", { method: "POST", body: await readBody(event) }),
)
