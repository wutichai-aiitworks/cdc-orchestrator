import { orchestratorFetch } from "~/server/utils/orchestratorFetch"
import { FEATURE_PAGES } from "~/utils/featurePages"

// The app renders in the browser (ssr: false), so only the server can answer a direct request for a
// switched-off page with a real HTTP 404. In-app navigation is covered by middleware/feature.ts.
export default defineEventHandler(async (event) => {
  const path = (event.path || "").split("?")[0].replace(/\/+$/, "") || "/"
  const feature = FEATURE_PAGES[path]
  if (!feature) return
  const cap = await orchestratorFetch<{ features?: Record<string, unknown> }>(event, "/cdc/capability").catch(() => null)
  if (cap?.features?.[feature] === true) return
  throw createError({ statusCode: 404, statusMessage: "Page not found" })
})
