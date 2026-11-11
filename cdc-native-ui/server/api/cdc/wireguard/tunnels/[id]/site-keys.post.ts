import { orchestratorFetch } from "~/server/utils/orchestratorFetch"

// New site key pair for an existing tunnel; the response carries the complete wg0.conf once.
export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(String(getRouterParam(event, "id")))
  const body = (await readBody(event).catch(() => null)) ?? {}
  return orchestratorFetch(event, `/cdc/wireguard/tunnels/${id}/site-keys`, { method: "POST", body })
})
