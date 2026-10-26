import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler((event) =>
  orchestratorFetch(event, "/cdc/pgloader/profiles", { method: "POST", body: readBody(event) }),
)
