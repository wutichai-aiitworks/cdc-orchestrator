import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  return orchestratorFetch(event, "/cdc/profiles/catalog/target", {
    method: "POST",
    body: await readBody(event),
  })
})
