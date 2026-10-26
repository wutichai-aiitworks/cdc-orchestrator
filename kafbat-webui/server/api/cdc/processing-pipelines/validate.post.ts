import { orchestratorFetch } from "../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) =>
  orchestratorFetch(event, "/cdc/processing-pipelines/validate", {
    method: "POST",
    body: await readBody(event),
  })
)
