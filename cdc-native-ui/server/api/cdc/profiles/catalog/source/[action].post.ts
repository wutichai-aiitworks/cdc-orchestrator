import { orchestratorFetch } from "../../../../../utils/orchestratorFetch"

// POST /api/cdc/profiles/catalog/source/{table|readiness}
//   table     — inspect one source table
//   readiness — live CDC readiness (ADR 0019): read-only probes, never changes the database
// One file for both keeps Nitro's typed-route union small (useFetch hits TS2589 beyond it).
const ACTIONS = new Set(["table", "readiness"])

export default defineEventHandler(async (event) => {
  const action = String(getRouterParam(event, "action") || "")
  if (!ACTIONS.has(action)) throw createError({ statusCode: 404, statusMessage: "Not found" })
  return orchestratorFetch(event, `/cdc/profiles/catalog/source/${action}`, {
    method: "POST",
    body: await readBody(event),
  })
})
