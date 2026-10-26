import { orchestratorFetch } from "~/server/utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const toolId = getRouterParam(event, "toolId")
  return orchestratorFetch(event, `/cdc/migration-tools/${encodeURIComponent(String(toolId))}/contract`)
})
