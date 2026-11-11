import { orchestratorFetch } from "~/server/utils/orchestratorFetch"

export default defineEventHandler(async (event) =>
  orchestratorFetch(event, "/cdc/wireguard/tunnels", { method: "POST", body: await readBody(event) }),
)
