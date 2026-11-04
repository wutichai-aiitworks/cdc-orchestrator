import { orchestratorFetch } from "../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) =>
  orchestratorFetch(event, "/cdc/rsync/validate", { method: "POST", body: await readBody(event) }),
)
