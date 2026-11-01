import { expect, test } from "@playwright/test";
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
} from "./helpers";

test.describe.configure({ mode: "serial" });

const created = new Set<string>();
let demo = false;
const scratchSchema = `${RUN_PREFIX.replace(/-/g, "_")}_ddl`;

test.beforeAll(async ({ request }) => {
  demo = await demoSourceReachable(request);
});

test.afterAll(async ({ request }) => {
  for (const id of created) await request.delete(`${API}/compare/profiles/${id}`).catch(() => undefined);
  if (demo) {
    await request.post(`${API}/compare/target/exec-ddl`, {
      data: { target: compareConnection(db.targetPort), statement: `DROP SCHEMA IF EXISTS "${scratchSchema}" CASCADE` },
    });
  }
});

test.describe("compare — saved profiles CRUD", () => {
  let id = "";

  test("POST /compare/profiles creates", async ({ request }) => {
    const res = await request.post(`${API}/compare/profiles`, {
      data: {
        name: `${RUN_PREFIX}-cmp`,
        source: compareConnection(db.sourcePort),
        target: compareConnection(db.targetPort),
      },
    });
    await expectStatus(res, 200);
    const body = await json(res);
    id = body.id;
    created.add(id);
    expect(body.name).toBe(`${RUN_PREFIX}-cmp`);
  });

  test("GET /compare/profiles lists it, GET /:id returns it", async ({ request }) => {
    const list = await json(await request.get(`${API}/compare/profiles`));
    expect(list.profiles.some((p: any) => p.id === id)).toBe(true);
    await expectStatus(await request.get(`${API}/compare/profiles/${id}`), 200);
  });

  test("KNOWN BUG: compare profiles mask stored passwords", async ({ request }) => {
    test.fail(true, "compare profiles return plaintext passwords (CDC profiles return ***) — see docs/testing/api-test-plan.md");
    const body = await json(await request.get(`${API}/compare/profiles/${id}`));
    expect(body.source.password).toBe("***");
    expect(body.target.password).toBe("***");
  });

  test("PUT /compare/profiles/:id renames", async ({ request }) => {
    const res = await request.put(`${API}/compare/profiles/${id}`, {
      data: {
        name: `${RUN_PREFIX}-cmp-renamed`,
        source: compareConnection(db.sourcePort),
        target: compareConnection(db.targetPort),
      },
    });
    await expectStatus(res, 200);
    expect((await json(res)).name).toBe(`${RUN_PREFIX}-cmp-renamed`);
  });

  test("DELETE /compare/profiles/:id then 404", async ({ request }) => {
    await expectStatus(await request.delete(`${API}/compare/profiles/${id}`), 200);
    created.delete(id);
    await expectStatus(await request.get(`${API}/compare/profiles/${id}`), 404);
  });
});

test.describe("compare — counts, preview, DDL", () => {
  test.beforeEach(() => test.skip(!demo, DEMO_UNREACHABLE));

  test("POST /compare/table/count returns exact count", async ({ request }) => {
    const res = await request.post(`${API}/compare/table/count`, {
      data: { connection: compareConnection(db.sourcePort), schema: db.schema, table: db.smallTable },
    });
    await expectStatus(res, 200);
    const body = await json(res);
    expect(body).toMatchObject({ schema: db.schema, table: db.smallTable });
    expect(body.count).toBeGreaterThan(0);
  });

  test("POST /compare/table/count on a missing table → 400", async ({ request }) => {
    const res = await request.post(`${API}/compare/table/count`, {
      data: { connection: compareConnection(db.sourcePort), schema: db.schema, table: "pw_no_such_table" },
    });
    await expectError(res, 400, /does not exist/);
  });

  test("table names are quoted identifiers (no SQL injection)", async ({ request }) => {
    const res = await request.post(`${API}/compare/table/count`, {
      data: {
        connection: compareConnection(db.sourcePort),
        schema: db.schema,
        table: `${db.smallTable}; DROP TABLE pw_x`,
      },
    });
    await expectError(res, 400, /does not exist/);
  });

  test("POST /compare/table/preview pages rows", async ({ request }) => {
    const res = await request.post(`${API}/compare/table/preview`, {
      data: { connection: compareConnection(db.sourcePort), schema: db.schema, table: db.smallTable, page_size: 2 },
    });
    await expectStatus(res, 200);
    const body = await json(res);
    expect(body.columns.length).toBeGreaterThan(0);
    expect(body.rows.length).toBe(2);
    expect(body.rows[0].length).toBe(body.columns.length);
    expect(body).toMatchObject({ page: 0, page_size: 2, has_more: true });
  });

  test("POST /compare/schema/counts covers the small table", async ({ request }) => {
    const res = await request.post(`${API}/compare/schema/counts`, {
      data: { connection: compareConnection(db.sourcePort), schema: db.schema },
    });
    await expectStatus(res, 200);
    const body = await json(res);
    expect(body.schema).toBe(db.schema);
    expect(body.counts[db.smallTable]).toBeGreaterThan(0);
  });

  test("POST /compare/target/exec-ddl creates and drops a scratch schema", async ({ request }) => {
    const target = compareConnection(db.targetPort);
    await expectStatus(
      await request.post(`${API}/compare/target/exec-ddl`, {
        data: { target, statement: `CREATE SCHEMA IF NOT EXISTS "${scratchSchema}"` },
      }),
      200
    );
    const catalog = await json(
      await request.post(`${API}/profiles/catalog/target`, {
        data: { target_type: "postgres", connection: { ...target, type: undefined } },
      })
    );
    expect(catalog.schemas.map((s: any) => s.name ?? s)).toContain(scratchSchema);

    await expectStatus(
      await request.post(`${API}/compare/target/exec-ddl`, {
        data: { target, statement: `DROP SCHEMA "${scratchSchema}"` },
      }),
      200
    );
  });

  test("POST /compare/target/exec-ddl rejects an empty statement", async ({ request }) => {
    const res = await request.post(`${API}/compare/target/exec-ddl`, {
      data: { target: compareConnection(db.targetPort), statement: "   " },
    });
    await expectError(res, 400, /statement is required/);
  });
});
