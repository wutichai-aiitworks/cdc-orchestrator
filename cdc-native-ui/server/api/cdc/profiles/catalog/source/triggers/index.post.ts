import { orchestratorFetch } from "../../../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  return orchestratorFetch(event, "/cdc/profiles/catalog/source/triggers", {
    method: "POST",
    body: await readBody(event),
  })
})
