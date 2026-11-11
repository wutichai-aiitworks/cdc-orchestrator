// Pages behind a feature switch (list: utils/featurePages.ts) exist only while the orchestrator reports
// the feature on — otherwise in-app navigation ends on a 404, like a page that does not exist. Off by
// default: loading errors also count as off. Direct requests get an HTTP 404 from the server
// (server/middleware/feature-pages.ts); the app renders in the browser (ssr: false).
import { FEATURE_PAGES } from "~/utils/featurePages"

export default defineNuxtRouteMiddleware(async (to) => {
  const feature = FEATURE_PAGES[to.path.replace(/\/+$/, "") || "/"]
  if (!feature) return
  const cap = await $fetch<{ features?: Record<string, unknown> }>("/api/cdc/capability").catch(() => null)
  if (cap?.features?.[feature] === true) return
  return abortNavigation(createError({ statusCode: 404, statusMessage: `Page not found: ${to.path}` }))
})
