import { orchestratorFetch } from "../../../utils/orchestratorFetch"

export default defineEventHandler((event) =>
  orchestratorFetch(event, "/cdc/pgloader/validate", { method: "POST", body: readBody(event) }),
)
