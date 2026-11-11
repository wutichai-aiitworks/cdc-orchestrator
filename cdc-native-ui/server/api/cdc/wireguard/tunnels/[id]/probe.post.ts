import { orchestratorFetch } from "~/server/utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(String(getRouterParam(event, "id")))
  return orchestratorFetch(event, `/cdc/wireguard/tunnels/${id}/probe`, { method: "POST", body: await readBody(event) })
})
