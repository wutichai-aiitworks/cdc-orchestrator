// node --test --import ./tests/unit/register-ts.mjs tests/unit/*.test.ts
import assert from "node:assert/strict"
import { test } from "node:test"

import { insightFromConfig } from "../../cdc-native-ui/utils/connectorMeta.ts"

const jdbcSource = (url: string) =>
  insightFromConfig({ "connector.class": "io.confluent.connect.jdbc.JdbcSourceConnector", "connection.url": url })

test("connector vendor: Oracle thin URLs are recognised (service, //service, SID)", () => {
  for (const [url, db] of [
    ["jdbc:oracle:thin:@10.0.0.5:1521/CRMSTG", "CRMSTG"],
    ["jdbc:oracle:thin:@//db.example:1522/PDB1", "PDB1"],
    ["jdbc:oracle:thin:@db.example:1521:ORCL", "ORCL"],
  ] as const) {
    const i = jdbcSource(url)
    assert.equal(i.databaseType, "oracle", url)
    assert.equal(i.databaseLabel, "Oracle")
    assert.equal(i.connection.database, db, url)
  }
  assert.equal(jdbcSource("jdbc:oracle:thin:@//db.example:1522/PDB1").connection.port, "1522")
})

test("connector vendor: Oracle TNS descriptor", () => {
  const i = jdbcSource(
    "jdbc:oracle:thin:@(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST=ora1)(PORT=1530))(CONNECT_DATA=(SERVICE_NAME=svc)))",
  )
  assert.equal(i.databaseType, "oracle")
  assert.deepEqual([i.connection.host, i.connection.port, i.connection.database], ["ora1", "1530", "svc"])
})

test("connector vendor: other engines unchanged, unknown stays unknown", () => {
  assert.equal(jdbcSource("jdbc:postgresql://h:5432/demo").databaseType, "postgres")
  assert.equal(jdbcSource("jdbc:foo://h/x").databaseType, "unknown")
})
