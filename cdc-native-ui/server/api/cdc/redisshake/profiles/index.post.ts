import { orchestratorFetch } from "~/server/utils/orchestratorFetch"

export default defineEventHandler(async (event) =>
  orchestratorFetch(event, "/cdc/redisshake/profiles", { method: "POST", body: await readBody(event) }),
)
