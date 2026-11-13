import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) =>
  orchestratorFetch(event, "/cdc/notifications/channels/test", { method: "POST", body: await readBody(event) }),
)
