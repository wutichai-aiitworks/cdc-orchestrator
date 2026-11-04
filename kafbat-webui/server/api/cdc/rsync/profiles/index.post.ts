import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) =>
  orchestratorFetch(event, "/cdc/rsync/profiles", { method: "POST", body: await readBody(event) }),
)
