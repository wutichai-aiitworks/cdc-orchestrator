import { orchestratorFetch } from "../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  return orchestratorFetch(event, "/cdc/connectors/templates")
})
