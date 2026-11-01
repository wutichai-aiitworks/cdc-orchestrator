import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler((event) =>
  orchestratorFetch(event, "/cdc/pgcopy/profiles", { method: "POST", body: readBody(event) }),
)
