// APP_MODE (cdc-native-ui/utils/appMode.ts): Production | Maintenance | Enhanced; same rules as src/config.rs.
import test from "node:test"
import assert from "node:assert/strict"

import { APP_MODE_RIBBON, parseAppMode, resolveAppMode } from "../../cdc-native-ui/utils/appMode.ts"

test("APP_MODE: the three modes in common spellings; other environment names are not modes", () => {
  for (const v of ["Production", "PRODUCTION", "prod"]) assert.equal(parseAppMode(v), "production", v)
  for (const v of ["MAINTENANCE", "Maintenance", " maintainance ", "Maintainance", "maint"]) assert.equal(parseAppMode(v), "maintenance", v)
  for (const v of ["ENHANCED", "Enhanced", "enhance", "Enhancement"]) assert.equal(parseAppMode(v), "enhanced", v)
  for (const v of ["dev", "sit", "uat", "", undefined, null]) assert.equal(parseAppMode(v), null, String(v))
})

test("APP_MODE resolves: APP_MODE, then retired ORCH_ENV, then production", () => {
  assert.equal(resolveAppMode("Enhanced", "prod"), "enhanced")
  assert.equal(resolveAppMode(undefined, "dev"), "enhanced")
  assert.equal(resolveAppMode(undefined, "prod"), "production")
  assert.equal(resolveAppMode("bogus"), "production")
  assert.equal(resolveAppMode(undefined), "production")
})

test("APP_MODE ribbon: none in production, one per other mode", () => {
  assert.equal(APP_MODE_RIBBON.production, undefined)
  assert.equal(APP_MODE_RIBBON.maintenance?.label, "Maintenance")
  assert.equal(APP_MODE_RIBBON.enhanced?.label, "Enhanced")
})
