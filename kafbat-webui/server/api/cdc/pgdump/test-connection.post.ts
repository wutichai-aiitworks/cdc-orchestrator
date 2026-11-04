import { orchestratorFetch } from "../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) =>
  orchestratorFetch(event, "/cdc/pgdump/test-connection", { method: "POST", body: await readBody(event) }),
)
