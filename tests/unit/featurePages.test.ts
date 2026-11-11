// Feature-switched pages (cdc-native-ui/utils/featurePages.ts).
import test from "node:test"
import assert from "node:assert/strict"

import { FEATURE_PAGES } from "../../cdc-native-ui/utils/featurePages.ts"

test("KSQL DB page exists only with ORCH_KSQL_ENABLED (capability ksql_enabled)", () => {
  assert.equal(FEATURE_PAGES["/ksqldb"], "ksql_enabled")
})
