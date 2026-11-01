import { expect, test } from "@playwright/test";
import { API, expectError, expectStatus, json } from "./helpers";

// Read-only ksqlDB checks. Statements that create/drop/terminate are not run here.

let configured = false;

test.beforeAll(async ({ request }) => {
  const res = await request.get(`${API}/ksqldb/info`);
  configured = res.ok() && (await res.json()).configured === true;
});

test.beforeEach(() => test.skip(!configured, "ksqlDB not configured on this orchestrator"));

test("GET /ksqldb/info returns server info", async ({ request }) => {
  const body = await json(await request.get(`${API}/ksqldb/info`));
  expect(body.info?.KsqlServerInfo?.kafkaClusterId).toBeTruthy();
});

for (const kind of ["streams", "tables", "queries"]) {
  test(`GET /ksqldb/${kind} returns an item list`, async ({ request }) => {
    const res = await request.get(`${API}/ksqldb/${kind}`);
    await expectStatus(res, 200);
    const body = await json(res);
    expect(body.configured).toBe(true);
    expect(Array.isArray(body.items)).toBe(true);
  });
}

test("POST /ksqldb/query runs SHOW STREAMS", async ({ request }) => {
  const res = await request.post(`${API}/ksqldb/query`, { data: { ksql: "SHOW STREAMS;" } });
  await expectStatus(res, 200);
  const body = await json(res);
  expect(body.kind).toBe("statement");
  expect(Array.isArray(body.result)).toBe(true);
});

test("POST /ksqldb/query surfaces syntax errors as 400", async ({ request }) => {
  await expectError(await request.post(`${API}/ksqldb/query`, { data: { ksql: "SELEKT nonsense;" } }), 400, /Syntax Error/);
});
