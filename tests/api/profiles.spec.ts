import { expect, test } from "@playwright/test";
import {
  API,
  DEMO_UNREACHABLE,
  RUN_PREFIX,
  compareConnection,
  db,
  demoSourceReachable,
  e2eEnabled,
  expectError,
  expectStatus,
  json,
  profileBody,
  profileConnection,
  sourceConfig,
  targetConfig,
  waitFor,
} from "./helpers";

test.describe.configure({ mode: "serial" });

const created = new Set<string>();
let demo = false;

test.beforeAll(async ({ request }) => {
  demo = await demoSourceReachable(request);
});

test.afterAll(async ({ request }) => {
  for (const id of created) {
    await request.post(`${API}/profiles/${id}/actions/stop`).catch(() => undefined);
    await request.delete(`${API}/profiles/${id}`).catch(() => undefined);
  }
});

async function createProfile(request: any, name: string, overrides: Record<string, unknown> = {}) {
  const res = await request.post(`${API}/profiles`, { data: profileBody(name, overrides) });
  await expectStatus(res, 200);
  const { profile } = await json(res);
  created.add(profile.id);
  return profile;
}

test.describe("profiles — connection validation & catalog", () => {
  test.beforeEach(() => test.skip(!demo, DEMO_UNREACHABLE));

  test("POST /profiles/validate/source accepts demo source", async ({ request }) => {
    const res = await request.post(`${API}/profiles/validate/source`, { data: sourceConfig() });
    await expectStatus(res, 200);
    const body = await json(res);
    expect(body.valid).toBe(true);
    expect(body.message).toContain(db.sourcePort);
  });

  test("POST /profiles/validate/source rejects wrong password with 400", async ({ request }) => {
    const res = await request.post(`${API}/profiles/validate/source`, {
      data: sourceConfig({ connection: profileConnection(db.sourcePort, "definitely-wrong") }),
    });
    await expectError(res, 400, /password authentication failed/i);
  });

  test("POST /profiles/validate/target accepts demo target", async ({ request }) => {
    const res = await request.post(`${API}/profiles/validate/target`, { data: targetConfig() });
    await expectStatus(res, 200);
    expect((await json(res)).valid).toBe(true);
  });

  test("POST /profiles/catalog/source lists schema and tables with PKs", async ({ request }) => {
    const res = await request.post(`${API}/profiles/catalog/source`, { data: sourceConfig() });
    await expectStatus(res, 200);
    const body = await json(res);
    expect(body.reachable).toBe(true);
    expect(body.schemas.map((s: any) => s.name)).toContain(db.schema);
    const table = body.tables.find((t: any) => t.schema === db.schema && t.name === db.smallTable);
    expect(table, `${db.schema}.${db.smallTable} in catalog`).toBeTruthy();
    expect(table.primary_key.length).toBeGreaterThan(0);
  });

  test("POST /profiles/catalog/source/table inspects columns", async ({ request }) => {
    const res = await request.post(`${API}/profiles/catalog/source/table`, {
      data: { source: sourceConfig(), schema: db.schema, name: db.smallTable, limit: 2 },
    });
    await expectStatus(res, 200);
    const body = await json(res);
    expect(body.name).toBe(db.smallTable);
    expect(body.columns.length).toBeGreaterThan(0);
    for (const pk of body.primary_key) {
      expect(body.columns.find((c: any) => c.name === pk)?.primary_key).toBe(true);
    }
  });

  test("POST /profiles/catalog/target lists target catalog", async ({ request }) => {
    const res = await request.post(`${API}/profiles/catalog/target`, { data: targetConfig() });
    await expectStatus(res, 200);
    expect(Array.isArray((await json(res)).schemas)).toBe(true);
  });

  test("POST /profiles/catalog/source/triggers/preview returns DDL without installing", async ({ request }) => {
    const table = `${db.schema}.${db.smallTable}`;
    const res = await request.post(`${API}/profiles/catalog/source/triggers/preview`, {
      data: { source: sourceConfig({ capture_mode: "trigger" }), tables: [table] },
    });
    await expectStatus(res, 200);
    const body = await json(res);
    expect(body.tables).toEqual([table]);
    expect(body.sql).toMatch(/CREATE TABLE IF NOT EXISTS "hyper_cdc"\."change_log"/);
    expect(body.sql).toMatch(/CREATE (OR REPLACE )?TRIGGER/i);
  });
});

test.describe("profiles — CRUD & lifecycle", () => {
  let profile: any;

  test("POST /profiles creates a draft with masked secrets", async ({ request }) => {
    profile = await createProfile(request, `${RUN_PREFIX}-crud`);
    expect(profile.id).toMatch(/^[0-9a-f-]{36}$/);
    expect(profile.status).toBe("draft");
    expect(profile.autostart).toBe(false);
    expect(profile.enabled).toBe(true);
    expect(profile.source.connection.password).toBe("***");
    expect(profile.target.connection.password).toBe("***");
    expect(profile.history.some((h: any) => h.event === "profile_created")).toBe(true);
  });

  test("GET /profiles lists it, GET /profiles/:id returns it", async ({ request }) => {
    const list = await json(await request.get(`${API}/profiles`));
    expect(list.profiles.some((p: any) => p.id === profile.id)).toBe(true);
    for (const p of list.profiles) {
      expect(p.source?.connection?.password ?? "***", `password leak in profile ${p.id}`).toBe("***");
    }
    const res = await request.get(`${API}/profiles/${profile.id}`);
    await expectStatus(res, 200);
    expect((await json(res)).profile.name).toBe(`${RUN_PREFIX}-crud`);
  });

  test("PUT /profiles/:id with masked password keeps the stored secret", async ({ request }) => {
    const body = profileBody(`${RUN_PREFIX}-crud-renamed`, {
      source: sourceConfig({ connection: profileConnection(db.sourcePort, "***") }),
      target: targetConfig({ connection: profileConnection(db.targetPort, "***") }),
    });
    const res = await request.put(`${API}/profiles/${profile.id}`, { data: body });
    await expectStatus(res, 200);
    expect((await json(res)).profile.name).toBe(`${RUN_PREFIX}-crud-renamed`);

    test.skip(!demo, DEMO_UNREACHABLE);
    const validate = await request.post(`${API}/profiles/validate/source`, {
      data: { ...sourceConfig({ connection: profileConnection(db.sourcePort, "***") }), profile_id: profile.id },
    });
    await expectStatus(validate, 200);
    expect((await json(validate)).valid, "masked password resolved via profile_id").toBe(true);
  });

  for (const sub of ["dlq", "investigate", "tables/sync-status", "logs"]) {
    test(`GET /profiles/:id/${sub} responds for a draft profile`, async ({ request }) => {
      const res = await request.get(`${API}/profiles/${profile.id}/${sub}`);
      await expectStatus(res, 200);
      await json(res);
    });
  }

  test("POST /profiles/:id/actions/:unknown → 400", async ({ request }) => {
    const res = await request.post(`${API}/profiles/${profile.id}/actions/explode`);
    await expectError(res, 400, /unsupported profile action/);
  });

  test("disabled profile refuses actions", async ({ request }) => {
    const off = await request.post(`${API}/profiles/${profile.id}/enable`, { data: { enabled: false } });
    await expectStatus(off, 200);
    expect((await json(off)).profile.enabled).toBe(false);

    const blocked = await request.post(`${API}/profiles/${profile.id}/actions/start`);
    await expectError(blocked, 400, /disabled/);
    // Not re-enabled here: enable=true currently starts the pipeline (known bug below),
    // which would leave connectors + a replication slot behind on the demo source.
  });

  test("KNOWN BUG: enabling a draft (autostart=false) profile does not start it", async ({ request }) => {
    test.fail(true, "set_profile_enabled(true) always calls start, ignoring autostart and Draft — see docs/testing/api-test-plan.md");
    // Unreachable source port: if the bug fires, start fails harmlessly instead of creating connectors.
    const draft = await createProfile(request, `${RUN_PREFIX}-enable-draft`, {
      source: sourceConfig({ connection: { ...profileConnection(db.sourcePort), port: "1" } }),
    });
    await expectStatus(await request.post(`${API}/profiles/${draft.id}/enable`, { data: { enabled: false } }), 200);
    const on = await request.post(`${API}/profiles/${draft.id}/enable`, { data: { enabled: true } });
    await expectStatus(on, 200);
    const after = (await json(on)).profile;
    expect(after.status).toBe("draft");
    expect(after.last_error).toBeNull();
  });

  test("DELETE /profiles/:id → 204, then 404, and it is audited", async ({ request }) => {
    const res = await request.delete(`${API}/profiles/${profile.id}`);
    await expectStatus(res, 204);
    created.delete(profile.id);
    await expectError(await request.get(`${API}/profiles/${profile.id}`), 404, /not found/);

    const audit = await request.get(`${API}/audit/events?profile_id=${profile.id}&limit=20`);
    await expectStatus(audit, 200);
    const actions = (await json(audit)).items.map((e: any) => e.action);
    expect(actions).toContain("profile.delete");
  });
});

test.describe("profiles — input validation", () => {
  test("GET /profiles/:id unknown → 404 envelope", async ({ request }) => {
    await expectError(await request.get(`${API}/profiles/00000000-0000-0000-0000-000000000000`), 404, /not found/);
  });

  test("GET /profiles/:id malformed uuid → 400", async ({ request }) => {
    await expectStatus(await request.get(`${API}/profiles/not-a-uuid`), 400);
  });

  test("POST /profiles missing required fields → 422", async ({ request }) => {
    await expectStatus(await request.post(`${API}/profiles`, { data: { source: {} } }), 422);
  });

  test("POST /profiles non-JSON body → 400", async ({ request }) => {
    const res = await request.post(`${API}/profiles`, {
      headers: { "content-type": "application/json" },
      data: Buffer.from("not json"),
    });
    await expectStatus(res, 400);
  });

  test("KNOWN BUG: body-parse errors use the JSON error envelope", async ({ request }) => {
    test.fail(true, "axum rejections return plain text, not { error } — see docs/testing/api-test-plan.md");
    await expectError(await request.post(`${API}/profiles`, { data: { source: {} } }), 422);
  });
});

test.describe("profiles — end-to-end replication (ORCH_TEST_E2E=true)", () => {
  test.skip(!e2eEnabled, "set ORCH_TEST_E2E=true to start pipelines and write to the demo target");
  test.setTimeout(180_000);

  const targetSchema = `${RUN_PREFIX.replace(/-/g, "_")}_e2e`;
  let profile: any;
  let sourceConnector = "";
  let sinkConnector = "";
  let slot = "";
  let topicPrefix = "";
  let sourceCount = 0;

  const connectorConfig = async (request: any, name: string) => {
    const res = await request.get(`/connect/connectors/${encodeURIComponent(name)}/config`);
    if (!res.ok()) return undefined;
    const body = await res.json();
    return (body.config ?? body) as Record<string, string>;
  };

  test.afterAll(async ({ request }) => {
    // The orchestrator only pauses connectors on delete (known bug), so remove them and the
    // replication slot directly; otherwise every run leaks WAL on the demo source.
    for (const name of [sourceConnector, sinkConnector].filter(Boolean)) {
      await request.delete(`/connect/connectors/${encodeURIComponent(name)}`).catch(() => undefined);
    }
    if (slot) {
      // exec-ddl returns no rows, so retry the idempotent drop while the slot goes inactive.
      for (let i = 0; i < 6; i++) {
        await new Promise((r) => setTimeout(r, 3000));
        await request.post(`${API}/compare/target/exec-ddl`, {
          data: {
            target: compareConnection(db.sourcePort),
            statement: `SELECT pg_drop_replication_slot(slot_name) FROM pg_replication_slots WHERE slot_name = '${slot}' AND NOT active`,
          },
        });
      }
    }
    if (topicPrefix) {
      const topics = await json(await request.get(`${API}/topics?search=${topicPrefix}&page_size=100`)).catch(() => ({ items: [] }));
      for (const t of topics.items ?? []) {
        if (String(t.name).startsWith(`${topicPrefix}.`)) await request.delete(`${API}/topics/${t.name}`);
      }
    }
    await request.post(`${API}/compare/target/exec-ddl`, {
      data: { target: compareConnection(db.targetPort), statement: `DROP SCHEMA IF EXISTS "${targetSchema}" CASCADE` },
    });
  });

  test("sync_once start creates RUNNING source + sink connectors", async ({ request }) => {
    test.skip(!demo, DEMO_UNREACHABLE);
    sourceCount = (
      await json(
        await request.post(`${API}/compare/table/count`, {
          data: { connection: compareConnection(db.sourcePort), schema: db.schema, table: db.smallTable },
        })
      )
    ).count;
    expect(sourceCount).toBeGreaterThan(0);

    profile = await createProfile(request, `${RUN_PREFIX}-e2e`, {
      source: sourceConfig({ capture_mode: "sync_once" }),
      flow: {
        transforms: [],
        schedule: { delivery_mode: "realtime" },
        mappings: [
          {
            source_schema: db.schema,
            source_table: db.smallTable,
            target_schema: targetSchema,
            target_table: db.smallTable,
            // identical=true means "same names as the source"; a remap must say false.
            identical: false,
            columns: [],
          },
        ],
      },
    });
    const started = await request.post(`${API}/profiles/${profile.id}/actions/start`);
    await expectStatus(started, 200);
    expect((await json(started)).profile.status).toBe("running");

    const inv = await json(await request.get(`${API}/profiles/${profile.id}/investigate`));
    sourceConnector = inv.profile.source_connector;
    sinkConnector = inv.profile.sink_connector;
    expect(sourceConnector).toBeTruthy();
    expect(sinkConnector).toBeTruthy();

    const src = await connectorConfig(request, sourceConnector);
    slot = src?.["slot.name"] ?? "";
    topicPrefix = src?.["topic.prefix"] ?? "";
    expect(topicPrefix).toBeTruthy();

    await waitFor(
      async () => {
        const fresh = await json(await request.get(`${API}/profiles/${profile.id}/investigate`));
        const states = ["source", "sink"].map((k) => fresh.connectors?.[k]?.status?.connector?.state);
        return states.every((s) => s === "RUNNING");
      },
      60_000,
      "source and sink connectors RUNNING"
    );
  });

  test("capture: the source topic receives every row of the table", async ({ request }) => {
    test.skip(!profile, "previous step did not start the pipeline");
    const topicName = `${topicPrefix}.${db.schema}.${db.smallTable}`;
    const count = await waitFor(
      async () => {
        const body = await json(await request.get(`${API}/topics?search=${topicName}`));
        const t = body.items.find((x: any) => x.name === topicName);
        return t && t.messages_count >= sourceCount ? t.messages_count : undefined;
      },
      90_000,
      `${topicName} to hold ${sourceCount} messages`
    );
    expect(count).toBe(sourceCount);
  });

  test("KNOWN BUG: sink writes to the mapped target schema (schema-qualified table)", async ({ request }) => {
    test.fail(true, "sink uses currentSchema + unqualified table.name.format; Debezium resolves an existing same-named table in another schema — see docs/testing/api-test-plan.md");
    test.skip(!profile, "previous step did not start the pipeline");
    const cfg = await connectorConfig(request, sinkConnector);
    expect(cfg?.["table.name.format"] ?? "").toMatch(new RegExp(`^"?${targetSchema}"?\\.`));
  });

  test("KNOWN BUG: sink sees new topics without the 5-minute regex metadata delay", async ({ request }) => {
    test.fail(true, "sink subscribes via topics.regex before topics exist; first data waits for metadata.max.age.ms (5 min) — see docs/testing/api-test-plan.md");
    test.skip(!profile, "previous step did not start the pipeline");
    const cfg = (await connectorConfig(request, sinkConnector)) ?? {};
    const fastRefresh = Number(cfg["consumer.override.metadata.max.age.ms"] ?? Infinity) <= 30_000;
    expect(Boolean(cfg["topics"]) || fastRefresh).toBe(true);
  });

  test.fixme("rows land in the mapped target schema", async () => {
    // Enable once the two sink bugs above are fixed: poll
    // POST /compare/table/count on { targetSchema, smallTable } until it equals sourceCount.
  });

  test("KNOWN BUG: deleting the profile removes its connectors", async ({ request }) => {
    test.fail(true, "delete_profile only pauses connectors when ORCH_STOP_DELETES_CONNECTOR=false (default), orphaning them and the replication slot — see docs/testing/api-test-plan.md");
    test.skip(!profile, "previous step did not start the pipeline");
    await expectStatus(await request.delete(`${API}/profiles/${profile.id}`), 204);
    created.delete(profile.id);
    await new Promise((r) => setTimeout(r, 3000));
    const listing = await (await request.get("/connect/connectors")).text();
    expect(listing).not.toContain(`"${sourceConnector}"`);
    expect(listing).not.toContain(`"${sinkConnector}"`);
  });
});

test.describe("profiles — test initial sync stops afterwards (ORCH_TEST_E2E=true)", () => {
  test.skip(!e2eEnabled, "set ORCH_TEST_E2E=true to start pipelines and write to the demo target");
  test.setTimeout(180_000);

  const schema = `${RUN_PREFIX.replace(/-/g, "_")}_ts`;
  const name = `${RUN_PREFIX}-testsync`;
  const target = compareConnection(db.targetPort);
  let profile: any;

  const ddl = (request: any, statement: string) =>
    request.post(`${API}/compare/target/exec-ddl`, { data: { target, statement } });
  const ownConnectors = async (request: any) =>
    ((await (await request.get("/connect/connectors")).text()).match(new RegExp(`"cdc-${name}[^"]*"`, "g")) ?? []).map(
      (n: string) => n.replace(/"/g, "")
    );
  const targetCount = async (request: any) =>
    (
      await json(
        await request.post(`${API}/compare/table/count`, { data: { connection: target, schema, table: "flights" } })
      )
    ).count as number;

  test.beforeAll(async ({ request }) => {
    test.skip(!(await demoSourceReachable(request)), DEMO_UNREACHABLE);
    // Target table with an extra `__deleted` column, as left by a Debezium sink — the copy
    // must still succeed (it copies shared columns by name).
    await expectStatus(
      await ddl(
        request,
        `CREATE SCHEMA IF NOT EXISTS "${schema}"; ` +
          `CREATE TABLE IF NOT EXISTS "${schema}".flights (LIKE bookings.flights INCLUDING DEFAULTS); ` +
          `ALTER TABLE "${schema}".flights ADD COLUMN IF NOT EXISTS __deleted text`
      ),
      200
    );
    profile = await createProfile(request, name, {
      source: sourceConfig({
        capture_mode: "incremental",
        tables: [`${db.schema}.flights`],
        incrementing_column: "flight_id",
        upsert_key_columns: ["flight_id"],
      }),
      flow: {
        transforms: [],
        schedule: { delivery_mode: "micro_batch", interval_seconds: 60 },
        mappings: [
          {
            source_schema: db.schema,
            source_table: "flights",
            target_schema: schema,
            target_table: "flights",
            identical: false,
            columns: [],
            pk_columns: ["flight_id"],
          },
        ],
      },
    });
  });

  test.afterAll(async ({ request }) => {
    if (profile) await request.delete(`${API}/profiles/${profile.id}`).catch(() => undefined);
    created.delete(profile?.id);
    for (const c of await ownConnectors(request)) await request.delete(`/connect/connectors/${c}`);
    const prefix = name.replace(/-/g, "_");
    const topics = await json(await request.get(`${API}/topics?search=${prefix}&page_size=50`)).catch(() => ({ items: [] }));
    for (const t of topics.items ?? []) await request.delete(`${API}/topics/${t.name}`);
    await ddl(request, `DROP SCHEMA IF EXISTS "${schema}" CASCADE`);
  });

  test("row_limit is rejected for logical (WAL) pipelines", async ({ request }) => {
    const logical = await createProfile(request, `${RUN_PREFIX}-testsync-logical`);
    const res = await request.post(`${API}/profiles/${logical.id}/actions/start?row_limit=5`);
    await expectError(res, 400, /row_limit applies only/);
  });

  test("copies the sample, then ends stopped with no connectors left", async ({ request }) => {
    const res = await request.post(`${API}/profiles/${profile.id}/actions/start?row_limit=5&clear_target=true`);
    await expectStatus(res, 200);
    const after = (await json(res)).profile;
    expect(after.status).toBe("stopped");
    expect(after.last_error).toBeNull();
    const last = after.history.at(-1);
    expect(last.event).toBe("profile_test_initial_sync");
    expect(last.detail).toContain(`${schema}.flights=5`);
    expect(last.detail).toMatch(/not a production sync/);

    expect(await ownConnectors(request)).toEqual([]);
    expect(await targetCount(request)).toBe(5);

    // Nothing keeps syncing: still stopped with exactly the sample a little later.
    await new Promise((r) => setTimeout(r, 10_000));
    expect((await json(await request.get(`${API}/profiles/${profile.id}`))).profile.status).toBe("stopped");
    expect(await targetCount(request)).toBe(5);

    const audit = await json(await request.get(`${API}/audit/events?profile_id=${profile.id}&limit=5`));
    expect(audit.items.some((e: any) => e.action === "profile.test_initial_sync" && e.outcome === "success")).toBe(true);
  });

  test("a failing test still removes its connectors and ends failed", async ({ request }) => {
    await expectStatus(await ddl(request, `DROP TABLE "${schema}".flights`), 200);
    const res = await request.post(`${API}/profiles/${profile.id}/actions/start?row_limit=5`);
    await expectError(res, 400, /does not exist/);
    const after = (await json(await request.get(`${API}/profiles/${profile.id}`))).profile;
    expect(after.status).toBe("failed");
    expect(after.history.at(-1).event).toBe("profile_test_initial_sync_failed");
    expect(await ownConnectors(request)).toEqual([]);
  });
});
