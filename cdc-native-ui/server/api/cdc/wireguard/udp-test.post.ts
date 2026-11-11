import { orchestratorFetch } from "~/server/utils/orchestratorFetch"

// Before a tunnel exists: { endpoint: "host:port" } → did test datagrams come back from the site's echo?
export default defineEventHandler(async (event) =>
  orchestratorFetch(event, "/cdc/wireguard/udp-test", { method: "POST", body: await readBody(event) }),
)
