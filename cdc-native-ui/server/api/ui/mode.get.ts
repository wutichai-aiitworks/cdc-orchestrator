import { resolveAppMode } from "~/utils/appMode"

// Public (the login page shows the ribbon too). Read at request time, so changing APP_MODE only
// needs a container restart, not a rebuild.
export default defineEventHandler(() => ({
  mode: resolveAppMode(process.env.APP_MODE, process.env.ORCH_ENV),
}))
