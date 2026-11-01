# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: compare.spec.ts >> compare — saved profiles CRUD >> KNOWN BUG: compare profiles mask stored passwords
- Location: tests/api/compare.spec.ts:57:7

# Error details

```
Error: expect(received).toBe(expected) // Object.is equality

Expected: "***"
Received: "demo"
```

# Test source

```ts
  1   | import { expect, test } from "@playwright/test";
  2   | import {
  3   |   API,
  4   |   DEMO_UNREACHABLE,
  5   |   RUN_PREFIX,
  6   |   compareConnection,
  7   |   db,
  8   |   demoSourceReachable,
  9   |   expectError,
  10  |   expectStatus,
  11  |   json,
  12  | } from "./helpers";
  13  | 
  14  | test.describe.configure({ mode: "serial" });
  15  | 
  16  | const created = new Set<string>();
  17  | let demo = false;
  18  | const scratchSchema = `${RUN_PREFIX.replace(/-/g, "_")}_ddl`;
  19  | 
  20  | test.beforeAll(async ({ request }) => {
  21  |   demo = await demoSourceReachable(request);
  22  | });
  23  | 
  24  | test.afterAll(async ({ request }) => {
  25  |   for (const id of created) await request.delete(`${API}/compare/profiles/${id}`).catch(() => undefined);
  26  |   if (demo) {
  27  |     await request.post(`${API}/compare/target/exec-ddl`, {
  28  |       data: { target: compareConnection(db.targetPort), statement: `DROP SCHEMA IF EXISTS "${scratchSchema}" CASCADE` },
  29  |     });
  30  |   }
  31  | });
  32  | 
  33  | test.describe("compare — saved profiles CRUD", () => {
  34  |   let id = "";
  35  | 
  36  |   test("POST /compare/profiles creates", async ({ request }) => {
  37  |     const res = await request.post(`${API}/compare/profiles`, {
  38  |       data: {
  39  |         name: `${RUN_PREFIX}-cmp`,
  40  |         source: compareConnection(db.sourcePort),
  41  |         target: compareConnection(db.targetPort),
  42  |       },
  43  |     });
  44  |     await expectStatus(res, 200);
  45  |     const body = await json(res);
  46  |     id = body.id;
  47  |     created.add(id);
  48  |     expect(body.name).toBe(`${RUN_PREFIX}-cmp`);
  49  |   });
  50  | 
  51  |   test("GET /compare/profiles lists it, GET /:id returns it", async ({ request }) => {
  52  |     const list = await json(await request.get(`${API}/compare/profiles`));
  53  |     expect(list.profiles.some((p: any) => p.id === id)).toBe(true);
  54  |     await expectStatus(await request.get(`${API}/compare/profiles/${id}`), 200);
  55  |   });
  56  | 
  57  |   test("KNOWN BUG: compare profiles mask stored passwords", async ({ request }) => {
  58  |     test.fail(true, "compare profiles return plaintext passwords (CDC profiles return ***) — see docs/testing/api-test-plan.md");
  59  |     const body = await json(await request.get(`${API}/compare/profiles/${id}`));
> 60  |     expect(body.source.password).toBe("***");
      |                                  ^ Error: expect(received).toBe(expected) // Object.is equality
  61  |     expect(body.target.password).toBe("***");
  62  |   });
  63  | 
  64  |   test("PUT /compare/profiles/:id renames", async ({ request }) => {
  65  |     const res = await request.put(`${API}/compare/profiles/${id}`, {
  66  |       data: {
  67  |         name: `${RUN_PREFIX}-cmp-renamed`,
  68  |         source: compareConnection(db.sourcePort),
  69  |         target: compareConnection(db.targetPort),
  70  |       },
  71  |     });
  72  |     await expectStatus(res, 200);
  73  |     expect((await json(res)).name).toBe(`${RUN_PREFIX}-cmp-renamed`);
  74  |   });
  75  | 
  76  |   test("DELETE /compare/profiles/:id then 404", async ({ request }) => {
  77  |     await expectStatus(await request.delete(`${API}/compare/profiles/${id}`), 200);
  78  |     created.delete(id);
  79  |     await expectStatus(await request.get(`${API}/compare/profiles/${id}`), 404);
  80  |   });
  81  | });
  82  | 
  83  | test.describe("compare — counts, preview, DDL", () => {
  84  |   test.beforeEach(() => test.skip(!demo, DEMO_UNREACHABLE));
  85  | 
  86  |   test("POST /compare/table/count returns exact count", async ({ request }) => {
  87  |     const res = await request.post(`${API}/compare/table/count`, {
  88  |       data: { connection: compareConnection(db.sourcePort), schema: db.schema, table: db.smallTable },
  89  |     });
  90  |     await expectStatus(res, 200);
  91  |     const body = await json(res);
  92  |     expect(body).toMatchObject({ schema: db.schema, table: db.smallTable });
  93  |     expect(body.count).toBeGreaterThan(0);
  94  |   });
  95  | 
  96  |   test("POST /compare/table/count on a missing table → 400", async ({ request }) => {
  97  |     const res = await request.post(`${API}/compare/table/count`, {
  98  |       data: { connection: compareConnection(db.sourcePort), schema: db.schema, table: "pw_no_such_table" },
  99  |     });
  100 |     await expectError(res, 400, /does not exist/);
  101 |   });
  102 | 
  103 |   test("table names are quoted identifiers (no SQL injection)", async ({ request }) => {
  104 |     const res = await request.post(`${API}/compare/table/count`, {
  105 |       data: {
  106 |         connection: compareConnection(db.sourcePort),
  107 |         schema: db.schema,
  108 |         table: `${db.smallTable}; DROP TABLE pw_x`,
  109 |       },
  110 |     });
  111 |     await expectError(res, 400, /does not exist/);
  112 |   });
  113 | 
  114 |   test("POST /compare/table/preview pages rows", async ({ request }) => {
  115 |     const res = await request.post(`${API}/compare/table/preview`, {
  116 |       data: { connection: compareConnection(db.sourcePort), schema: db.schema, table: db.smallTable, page_size: 2 },
  117 |     });
  118 |     await expectStatus(res, 200);
  119 |     const body = await json(res);
  120 |     expect(body.columns.length).toBeGreaterThan(0);
  121 |     expect(body.rows.length).toBe(2);
  122 |     expect(body.rows[0].length).toBe(body.columns.length);
  123 |     expect(body).toMatchObject({ page: 0, page_size: 2, has_more: true });
  124 |   });
  125 | 
  126 |   test("POST /compare/schema/counts covers the small table", async ({ request }) => {
  127 |     const res = await request.post(`${API}/compare/schema/counts`, {
  128 |       data: { connection: compareConnection(db.sourcePort), schema: db.schema },
  129 |     });
  130 |     await expectStatus(res, 200);
  131 |     const body = await json(res);
  132 |     expect(body.schema).toBe(db.schema);
  133 |     expect(body.counts[db.smallTable]).toBeGreaterThan(0);
  134 |   });
  135 | 
  136 |   test("POST /compare/target/exec-ddl creates and drops a scratch schema", async ({ request }) => {
  137 |     const target = compareConnection(db.targetPort);
  138 |     await expectStatus(
  139 |       await request.post(`${API}/compare/target/exec-ddl`, {
  140 |         data: { target, statement: `CREATE SCHEMA IF NOT EXISTS "${scratchSchema}"` },
  141 |       }),
  142 |       200
  143 |     );
  144 |     const catalog = await json(
  145 |       await request.post(`${API}/profiles/catalog/target`, {
  146 |         data: { target_type: "postgres", connection: { ...target, type: undefined } },
  147 |       })
  148 |     );
  149 |     expect(catalog.schemas.map((s: any) => s.name ?? s)).toContain(scratchSchema);
  150 | 
  151 |     await expectStatus(
  152 |       await request.post(`${API}/compare/target/exec-ddl`, {
  153 |         data: { target, statement: `DROP SCHEMA "${scratchSchema}"` },
  154 |       }),
  155 |       200
  156 |     );
  157 |   });
  158 | 
  159 |   test("POST /compare/target/exec-ddl rejects an empty statement", async ({ request }) => {
  160 |     const res = await request.post(`${API}/compare/target/exec-ddl`, {
```