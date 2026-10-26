import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) =>
  orchestratorFetch(event, "/cdc/notifications/rules", {
    method: "POST",
    body: await readBody(event),
  })
)
