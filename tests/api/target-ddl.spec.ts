import { expect, test } from "@playwright/test";
import { generateTargetDdl } from "../../kafbat-webui/utils/targetDdl";
import {
  API,
  DEMO_UNREACHABLE,
  RUN_PREFIX,
  compareConnection,
  db,
  demoSourceReachable,
  expectError,
  expectStatus,
  json,
  sourceConfig,
  targetConfig,
} from "./helpers";

// Mapping step "Create table": inspect source → generate target DDL → POST /profiles/catalog/target/ddl.
test.describe.configure({ mode: "serial" });

const scratchSchema = `${RUN_PREFIX.replace(/-/g, "_")}_ddl`;
let demo = false;

test.beforeAll(async ({ request }) => {
  demo = await demoSourceReachable(request);
});

test.afterAll(async ({ request }) => {
  if (demo) {
    // The create endpoint refuses DROP by design; clean up through the compare DDL endpoint.
    await request.post(`${API}/compare/target/exec-ddl`, {
      data: { target: compareConnection(db.targetPort), statement: `DROP SCHEMA IF EXISTS "${scratchSchema}" CASCADE` },
    });
  }
});

test.describe("target DDL — guard", () => {
  for (const [label, statement] of [
    ["DROP", `DROP TABLE "${scratchSchema}"."x"`],
    ["chained DROP", `CREATE TABLE "${scratchSchema}"."x" (id INT); DROP TABLE "${scratchSchema}"."y"`],
    ["ALTER", `ALTER TABLE "${scratchSchema}"."x" ADD c INT`],
    ["DML", `INSERT INTO "${scratchSchema}"."x" VALUES (1)`],
    ["Oracle wrapper hiding DROP", `BEGIN EXECUTE IMMEDIATE 'DROP TABLE X'; END;`],
  ]) {
    test(`rejects ${label} with 400 before connecting`, async ({ request }) => {
      const res = await request.post(`${API}/profiles/catalog/target/ddl`, {
        data: { target: targetConfig(), statements: [statement] },
      });
      await expectError(res, 400, /statement 1:/);
    });
  }

  test("rejects an empty statement list", async ({ request }) => {
    const res = await request.post(`${API}/profiles/catalog/target/ddl`, {
      data: { target: targetConfig(), statements: ["  "] },
    });
    await expectError(res, 400, /must not be empty/);
  });

  test("rejects non-database targets", async ({ request }) => {
    const res = await request.post(`${API}/profiles/catalog/target/ddl`, {
      data: { target: targetConfig({ target_type: "kafka" }), statements: ["CREATE TABLE t (id INT)"] },
    });
    await expectError(res, 400, /unsupported target_type/);
  });
});

test.describe("target DDL — round trip on the demo target", () => {
  test.beforeEach(() => test.skip(!demo, DEMO_UNREACHABLE));

  let statements: string[] = [];
  let sourcePk: string[] = [];
  let sourceIndexCount = 0;

  test("inspect reports column sizes and index metadata", async ({ request }) => {
    const res = await request.post(`${API}/profiles/catalog/source/table`, {
      data: { source: sourceConfig(), schema: db.schema, name: db.smallTable, limit: 0 },
    });
    await expectStatus(res, 200);
    const body = await json(res);
    expect(Array.isArray(body.indexes), "indexes reported (cdc-jdbc-admin with index metadata)").toBe(true);
    sourcePk = body.primary_key;
    expect(sourcePk.length).toBeGreaterThan(0);
    const generated = generateTargetDdl({
      sourceDialect: "postgres",
      targetDialect: "postgres",
      targetSchema: scratchSchema,
      targetTable: db.smallTable,
      columns: body.columns,
      primaryKey: body.primary_key,
      indexes: body.indexes,
    });
    statements = generated.statements;
    sourceIndexCount = statements.filter((s) => /\bINDEX\b/.test(s)).length;
    expect(statements[1]).toMatch(/^CREATE TABLE IF NOT EXISTS /);
    expect(statements[1]).toContain(`PRIMARY KEY (${sourcePk.map((c) => `"${c}"`).join(", ")})`);
  });

  test("creates the table with the same primary key and indexes, and is safe to re-run", async ({ request }) => {
    test.skip(!statements.length, "previous step did not generate DDL");
    for (const attempt of ["first run", "re-run"]) {
      const res = await request.post(`${API}/profiles/catalog/target/ddl`, {
        data: { target: targetConfig(), statements },
      });
      await expectStatus(res, 200);
      const body = await json(res);
      expect(body.status, `${attempt}: ${JSON.stringify(body.results)}`).toBe(true);
      expect(body.executed).toBe(statements.length);
    }

    const res = await request.post(`${API}/profiles/catalog/target/table`, {
      data: { target: targetConfig(), schema: scratchSchema, name: db.smallTable, limit: 0 },
    });
    await expectStatus(res, 200);
    const target = await json(res);
    expect(target.primary_key).toEqual(sourcePk);
    const nonPkIndexes = (target.indexes ?? []).filter(
      (ix: any) => !(ix.unique && ix.columns.join(",") === sourcePk.join(",")),
    );
    expect(nonPkIndexes.length).toBe(sourceIndexCount);
  });
});
