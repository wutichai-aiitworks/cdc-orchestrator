import { resolveAppMode } from "~/utils/appMode"

// Public (the login page shows the ribbon too). Read per request: changing APP_MODE needs a restart, not a rebuild.
// If your auth middleware guards /api/**, allow-list this path.
export default defineEventHandler(() => ({
  mode: resolveAppMode(process.env.APP_MODE, process.env.APP_ENV),
}))
