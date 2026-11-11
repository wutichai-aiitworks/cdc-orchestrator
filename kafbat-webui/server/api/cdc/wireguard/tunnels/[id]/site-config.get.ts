import { orchestratorFetch } from "~/server/utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const id = encodeURIComponent(String(getRouterParam(event, "id")))
  return orchestratorFetch(event, `/cdc/wireguard/tunnels/${id}/site-config`)
})
