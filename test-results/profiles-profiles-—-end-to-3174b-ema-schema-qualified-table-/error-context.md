# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: profiles.spec.ts >> profiles — end-to-end replication (ORCH_TEST_E2E=true) >> KNOWN BUG: sink writes to the mapped target schema (schema-qualified table)
- Location: tests/api/profiles.spec.ts:349:7

# Error details

```
Error: expect(received).toMatch(expected)

Expected pattern: /^"?pw_mvxizvb1_e2e"?\./
Received string:  "${topic}"
```

# Test source

```ts
  253 |     for (const name of [sourceConnector, sinkConnector].filter(Boolean)) {
  254 |       await request.delete(`/connect/connectors/${encodeURIComponent(name)}`).catch(() => undefined);
  255 |     }
  256 |     if (slot) {
  257 |       // exec-ddl returns no rows, so retry the idempotent drop while the slot goes inactive.
  258 |       for (let i = 0; i < 6; i++) {
  259 |         await new Promise((r) => setTimeout(r, 3000));
  260 |         await request.post(`${API}/compare/target/exec-ddl`, {
  261 |           data: {
  262 |             target: compareConnection(db.sourcePort),
  263 |             statement: `SELECT pg_drop_replication_slot(slot_name) FROM pg_replication_slots WHERE slot_name = '${slot}' AND NOT active`,
  264 |           },
  265 |         });
  266 |       }
  267 |     }
  268 |     if (topicPrefix) {
  269 |       const topics = await json(await request.get(`${API}/topics?search=${topicPrefix}&page_size=100`)).catch(() => ({ items: [] }));
  270 |       for (const t of topics.items ?? []) {
  271 |         if (String(t.name).startsWith(`${topicPrefix}.`)) await request.delete(`${API}/topics/${t.name}`);
  272 |       }
  273 |     }
  274 |     await request.post(`${API}/compare/target/exec-ddl`, {
  275 |       data: { target: compareConnection(db.targetPort), statement: `DROP SCHEMA IF EXISTS "${targetSchema}" CASCADE` },
  276 |     });
  277 |   });
  278 | 
  279 |   test("sync_once start creates RUNNING source + sink connectors", async ({ request }) => {
  280 |     test.skip(!demo, DEMO_UNREACHABLE);
  281 |     sourceCount = (
  282 |       await json(
  283 |         await request.post(`${API}/compare/table/count`, {
  284 |           data: { connection: compareConnection(db.sourcePort), schema: db.schema, table: db.smallTable },
  285 |         })
  286 |       )
  287 |     ).count;
  288 |     expect(sourceCount).toBeGreaterThan(0);
  289 | 
  290 |     profile = await createProfile(request, `${RUN_PREFIX}-e2e`, {
  291 |       source: sourceConfig({ capture_mode: "sync_once" }),
  292 |       flow: {
  293 |         transforms: [],
  294 |         schedule: { delivery_mode: "realtime" },
  295 |         mappings: [
  296 |           {
  297 |             source_schema: db.schema,
  298 |             source_table: db.smallTable,
  299 |             target_schema: targetSchema,
  300 |             target_table: db.smallTable,
  301 |             // identical=true means "same names as the source"; a remap must say false.
  302 |             identical: false,
  303 |             columns: [],
  304 |           },
  305 |         ],
  306 |       },
  307 |     });
  308 |     const started = await request.post(`${API}/profiles/${profile.id}/actions/start`);
  309 |     await expectStatus(started, 200);
  310 |     expect((await json(started)).profile.status).toBe("running");
  311 | 
  312 |     const inv = await json(await request.get(`${API}/profiles/${profile.id}/investigate`));
  313 |     sourceConnector = inv.profile.source_connector;
  314 |     sinkConnector = inv.profile.sink_connector;
  315 |     expect(sourceConnector).toBeTruthy();
  316 |     expect(sinkConnector).toBeTruthy();
  317 | 
  318 |     const src = await connectorConfig(request, sourceConnector);
  319 |     slot = src?.["slot.name"] ?? "";
  320 |     topicPrefix = src?.["topic.prefix"] ?? "";
  321 |     expect(topicPrefix).toBeTruthy();
  322 | 
  323 |     await waitFor(
  324 |       async () => {
  325 |         const fresh = await json(await request.get(`${API}/profiles/${profile.id}/investigate`));
  326 |         const states = ["source", "sink"].map((k) => fresh.connectors?.[k]?.status?.connector?.state);
  327 |         return states.every((s) => s === "RUNNING");
  328 |       },
  329 |       60_000,
  330 |       "source and sink connectors RUNNING"
  331 |     );
  332 |   });
  333 | 
  334 |   test("capture: the source topic receives every row of the table", async ({ request }) => {
  335 |     test.skip(!profile, "previous step did not start the pipeline");
  336 |     const topicName = `${topicPrefix}.${db.schema}.${db.smallTable}`;
  337 |     const count = await waitFor(
  338 |       async () => {
  339 |         const body = await json(await request.get(`${API}/topics?search=${topicName}`));
  340 |         const t = body.items.find((x: any) => x.name === topicName);
  341 |         return t && t.messages_count >= sourceCount ? t.messages_count : undefined;
  342 |       },
  343 |       90_000,
  344 |       `${topicName} to hold ${sourceCount} messages`
  345 |     );
  346 |     expect(count).toBe(sourceCount);
  347 |   });
  348 | 
  349 |   test("KNOWN BUG: sink writes to the mapped target schema (schema-qualified table)", async ({ request }) => {
  350 |     test.fail(true, "sink uses currentSchema + unqualified table.name.format; Debezium resolves an existing same-named table in another schema — see docs/testing/api-test-plan.md");
  351 |     test.skip(!profile, "previous step did not start the pipeline");
  352 |     const cfg = await connectorConfig(request, sinkConnector);
> 353 |     expect(cfg?.["table.name.format"] ?? "").toMatch(new RegExp(`^"?${targetSchema}"?\\.`));
      |                                              ^ Error: expect(received).toMatch(expected)
  354 |   });
  355 | 
  356 |   test("KNOWN BUG: sink sees new topics without the 5-minute regex metadata delay", async ({ request }) => {
  357 |     test.fail(true, "sink subscribes via topics.regex before topics exist; first data waits for metadata.max.age.ms (5 min) — see docs/testing/api-test-plan.md");
  358 |     test.skip(!profile, "previous step did not start the pipeline");
  359 |     const cfg = (await connectorConfig(request, sinkConnector)) ?? {};
  360 |     const fastRefresh = Number(cfg["consumer.override.metadata.max.age.ms"] ?? Infinity) <= 30_000;
  361 |     expect(Boolean(cfg["topics"]) || fastRefresh).toBe(true);
  362 |   });
  363 | 
  364 |   test.fixme("rows land in the mapped target schema", async () => {
  365 |     // Enable once the two sink bugs above are fixed: poll
  366 |     // POST /compare/table/count on { targetSchema, smallTable } until it equals sourceCount.
  367 |   });
  368 | 
  369 |   test("KNOWN BUG: deleting the profile removes its connectors", async ({ request }) => {
  370 |     test.fail(true, "delete_profile only pauses connectors when ORCH_STOP_DELETES_CONNECTOR=false (default), orphaning them and the replication slot — see docs/testing/api-test-plan.md");
  371 |     test.skip(!profile, "previous step did not start the pipeline");
  372 |     await expectStatus(await request.delete(`${API}/profiles/${profile.id}`), 204);
  373 |     created.delete(profile.id);
  374 |     await new Promise((r) => setTimeout(r, 3000));
  375 |     const listing = await (await request.get("/connect/connectors")).text();
  376 |     expect(listing).not.toContain(`"${sourceConnector}"`);
  377 |     expect(listing).not.toContain(`"${sinkConnector}"`);
  378 |   });
  379 | });
  380 | 
  381 | test.describe("profiles — test initial sync stops afterwards (ORCH_TEST_E2E=true)", () => {
  382 |   test.skip(!e2eEnabled, "set ORCH_TEST_E2E=true to start pipelines and write to the demo target");
  383 |   test.setTimeout(180_000);
  384 | 
  385 |   const schema = `${RUN_PREFIX.replace(/-/g, "_")}_ts`;
  386 |   const name = `${RUN_PREFIX}-testsync`;
  387 |   const target = compareConnection(db.targetPort);
  388 |   let profile: any;
  389 | 
  390 |   const ddl = (request: any, statement: string) =>
  391 |     request.post(`${API}/compare/target/exec-ddl`, { data: { target, statement } });
  392 |   const ownConnectors = async (request: any) =>
  393 |     ((await (await request.get("/connect/connectors")).text()).match(new RegExp(`"cdc-${name}[^"]*"`, "g")) ?? []).map(
  394 |       (n: string) => n.replace(/"/g, "")
  395 |     );
  396 |   const targetCount = async (request: any) =>
  397 |     (
  398 |       await json(
  399 |         await request.post(`${API}/compare/table/count`, { data: { connection: target, schema, table: "flights" } })
  400 |       )
  401 |     ).count as number;
  402 | 
  403 |   test.beforeAll(async ({ request }) => {
  404 |     test.skip(!(await demoSourceReachable(request)), DEMO_UNREACHABLE);
  405 |     // Target table with an extra `__deleted` column, as left by a Debezium sink — the copy
  406 |     // must still succeed (it copies shared columns by name).
  407 |     await expectStatus(
  408 |       await ddl(
  409 |         request,
  410 |         `CREATE SCHEMA IF NOT EXISTS "${schema}"; ` +
  411 |           `CREATE TABLE IF NOT EXISTS "${schema}".flights (LIKE bookings.flights INCLUDING DEFAULTS); ` +
  412 |           `ALTER TABLE "${schema}".flights ADD COLUMN IF NOT EXISTS __deleted text`
  413 |       ),
  414 |       200
  415 |     );
  416 |     profile = await createProfile(request, name, {
  417 |       source: sourceConfig({
  418 |         capture_mode: "incremental",
  419 |         tables: [`${db.schema}.flights`],
  420 |         incrementing_column: "flight_id",
  421 |         upsert_key_columns: ["flight_id"],
  422 |       }),
  423 |       flow: {
  424 |         transforms: [],
  425 |         schedule: { delivery_mode: "micro_batch", interval_seconds: 60 },
  426 |         mappings: [
  427 |           {
  428 |             source_schema: db.schema,
  429 |             source_table: "flights",
  430 |             target_schema: schema,
  431 |             target_table: "flights",
  432 |             identical: false,
  433 |             columns: [],
  434 |             pk_columns: ["flight_id"],
  435 |           },
  436 |         ],
  437 |       },
  438 |     });
  439 |   });
  440 | 
  441 |   test.afterAll(async ({ request }) => {
  442 |     if (profile) await request.delete(`${API}/profiles/${profile.id}`).catch(() => undefined);
  443 |     created.delete(profile?.id);
  444 |     for (const c of await ownConnectors(request)) await request.delete(`/connect/connectors/${c}`);
  445 |     const prefix = name.replace(/-/g, "_");
  446 |     const topics = await json(await request.get(`${API}/topics?search=${prefix}&page_size=50`)).catch(() => ({ items: [] }));
  447 |     for (const t of topics.items ?? []) await request.delete(`${API}/topics/${t.name}`);
  448 |     await ddl(request, `DROP SCHEMA IF EXISTS "${schema}" CASCADE`);
  449 |   });
  450 | 
  451 |   test("row_limit is rejected for logical (WAL) pipelines", async ({ request }) => {
  452 |     const logical = await createProfile(request, `${RUN_PREFIX}-testsync-logical`);
  453 |     const res = await request.post(`${API}/profiles/${logical.id}/actions/start?row_limit=5`);
```