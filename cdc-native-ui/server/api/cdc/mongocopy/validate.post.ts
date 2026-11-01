import { orchestratorFetch } from "../../../utils/orchestratorFetch"

export default defineEventHandler((event) =>
  orchestratorFetch(event, "/cdc/mongocopy/validate", { method: "POST", body: readBody(event) }),
)
