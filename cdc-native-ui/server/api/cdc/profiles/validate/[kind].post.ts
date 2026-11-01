import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const kind = String(getRouterParam(event, "kind") || "")
  if (!["source", "target"].includes(kind)) {
    throw createError({ statusCode: 400, statusMessage: "Unsupported validation target" })
  }
  return orchestratorFetch(event, `/cdc/profiles/validate/${kind}`, {
    method: "POST",
    body: await readBody(event),
  })
})
