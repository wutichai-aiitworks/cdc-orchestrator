import { orchestratorFetch } from "../../../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const name = encodeURIComponent(getRouterParam(event, "name") || "")
  const task = Number(getRouterParam(event, "task") || "")
  if (!name) throw createError({ statusCode: 400, statusMessage: "name is required" })
  if (!Number.isFinite(task) || task < 0) {
    throw createError({ statusCode: 400, statusMessage: "task is required" })
  }
  return orchestratorFetch(event, `/cdc/connectors/${name}/tasks/${task}/restart`, {
    method: "POST",
  })
})
