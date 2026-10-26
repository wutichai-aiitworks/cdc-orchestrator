import { orchestratorFetch } from "../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  setHeader(event, "content-type", "text/plain; charset=utf-8")
  return orchestratorFetch<string>(event, "/cdc/metrics", {
    responseType: "text",
  })
})
