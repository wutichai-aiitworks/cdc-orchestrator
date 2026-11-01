# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: profiles.spec.ts >> profiles — input validation >> KNOWN BUG: body-parse errors use the JSON error envelope
- Location: tests/api/profiles.spec.ts:225:7

# Error details

```
Error: expected JSON from http://localhost:9788/cdc/profiles [422], got: Failed to deserialize the JSON body into the target type: source: missing field `source_type` at line 1 column 12
```

# Test source

```ts
  1   | import { expect, type APIRequestContext, type APIResponse } from "@playwright/test";
  2   | 
  3   | /**
  4   |  * Shared config for the strict API suites.
  5   |  *
  6   |  * Every fixture a suite creates is prefixed with `RUN_PREFIX` and deleted in
  7   |  * afterAll — the stack under test may also hold real customer profiles, so
  8   |  * suites must never read-modify or delete anything they did not create.
  9   |  *
  10  |  * Env:
  11  |  *   ORCH_TEST_BASE_URL        orchestrator (default http://localhost:9788)
  12  |  *   ORCH_TEST_TOKEN           Bearer token when ORCH_API_TOKEN is set
  13  |  *   ORCH_TEST_DB_HOST         host the orchestrator/jdbc-admin use to reach the demo DBs
  14  |  *                             (default host.docker.internal — the demo Postgres containers
  15  |  *                             sit on a different compose network than the orchestrator)
  16  |  *   ORCH_TEST_SOURCE_PORT     demo source port (default 25432)
  17  |  *   ORCH_TEST_TARGET_PORT     demo target port (default 25433)
  18  |  *   ORCH_TEST_DB_NAME / _USER / _PASSWORD   (default demo / demo / demo)
  19  |  *   ORCH_TEST_SCHEMA          demo schema (default bookings)
  20  |  *   ORCH_TEST_E2E=true        enable tests that start pipelines and write to the target
  21  |  */
  22  | 
  23  | export const API = "/cdc";
  24  | 
  25  | export const RUN_PREFIX = `pw-${Date.now().toString(36)}`;
  26  | 
  27  | const env = (key: string, fallback: string) => process.env[key]?.trim() || fallback;
  28  | 
  29  | export const db = {
  30  |   host: env("ORCH_TEST_DB_HOST", "host.docker.internal"),
  31  |   sourcePort: env("ORCH_TEST_SOURCE_PORT", "25432"),
  32  |   targetPort: env("ORCH_TEST_TARGET_PORT", "25433"),
  33  |   database: env("ORCH_TEST_DB_NAME", "demo"),
  34  |   username: env("ORCH_TEST_DB_USER", "demo"),
  35  |   password: env("ORCH_TEST_DB_PASSWORD", "demo"),
  36  |   schema: env("ORCH_TEST_SCHEMA", "bookings"),
  37  |   /** Small table (10 rows in the demo dataset) used for previews and replication. */
  38  |   smallTable: env("ORCH_TEST_SMALL_TABLE", "airplanes_data"),
  39  | };
  40  | 
  41  | export const e2eEnabled = process.env.ORCH_TEST_E2E === "true";
  42  | 
  43  | /** Connection map in the CDC-profile shape (`source.connection` / `target.connection`). */
  44  | export function profileConnection(port: string, password = db.password) {
  45  |   return {
  46  |     host: db.host,
  47  |     port,
  48  |     database: db.database,
  49  |     username: db.username,
  50  |     password,
  51  |   };
  52  | }
  53  | 
  54  | /** Connection in the schema-compare shape (`SchemaCompareConn`). */
  55  | export function compareConnection(port: string) {
  56  |   return { type: "postgres", ...profileConnection(port) };
  57  | }
  58  | 
  59  | export function sourceConfig(overrides: Record<string, unknown> = {}) {
  60  |   return {
  61  |     source_type: "postgres",
  62  |     connection: profileConnection(db.sourcePort),
  63  |     capture_mode: "logical",
  64  |     schema_mode: "specific_schemas",
  65  |     table_mode: "specific_tables",
  66  |     schemas: [db.schema],
  67  |     tables: [`${db.schema}.${db.smallTable}`],
  68  |     ...overrides,
  69  |   };
  70  | }
  71  | 
  72  | export function targetConfig(overrides: Record<string, unknown> = {}) {
  73  |   return {
  74  |     name: `${RUN_PREFIX}-target`,
  75  |     target_type: "postgres",
  76  |     connection: profileConnection(db.targetPort),
  77  |     schemas: [],
  78  |     ...overrides,
  79  |   };
  80  | }
  81  | 
  82  | export function profileBody(name: string, overrides: Record<string, unknown> = {}) {
  83  |   return {
  84  |     name,
  85  |     autostart: false,
  86  |     pattern: "source_to_target",
  87  |     source: sourceConfig(),
  88  |     target: targetConfig(),
  89  |     flow: { transforms: [], schedule: { delivery_mode: "realtime" }, mappings: [] },
  90  |     ...overrides,
  91  |   };
  92  | }
  93  | 
  94  | export async function json<T = any>(response: APIResponse): Promise<T> {
  95  |   const text = await response.text();
  96  |   try {
  97  |     return JSON.parse(text) as T;
  98  |   } catch {
> 99  |     throw new Error(`expected JSON from ${response.url()} [${response.status()}], got: ${text.slice(0, 300)}`);
      |           ^ Error: expected JSON from http://localhost:9788/cdc/profiles [422], got: Failed to deserialize the JSON body into the target type: source: missing field `source_type` at line 1 column 12
  100 |   }
  101 | }
  102 | 
  103 | /** Assert an exact status and include the body in the failure message. */
  104 | export async function expectStatus(response: APIResponse, status: number) {
  105 |   if (response.status() !== status) {
  106 |     const body = (await response.text()).slice(0, 500);
  107 |     expect(response.status(), `${response.url()} → ${body}`).toBe(status);
  108 |   }
  109 | }
  110 | 
  111 | /** Assert the orchestrator's error envelope `{ "error": "..." }`. */
  112 | export async function expectError(response: APIResponse, status: number, match?: RegExp) {
  113 |   await expectStatus(response, status);
  114 |   const body = await json<{ error?: string }>(response);
  115 |   expect(typeof body.error, "error envelope").toBe("string");
  116 |   if (match) expect(body.error).toMatch(match);
  117 | }
  118 | 
  119 | /** True when the orchestrator can reach the demo source DB (gates DB-backed tests). */
  120 | export async function demoSourceReachable(request: APIRequestContext): Promise<boolean> {
  121 |   const res = await request.post(`${API}/profiles/validate/source`, { data: sourceConfig() });
  122 |   if (!res.ok()) return false;
  123 |   const body = await res.json().catch(() => ({}));
  124 |   return body?.valid === true;
  125 | }
  126 | 
  127 | export const DEMO_UNREACHABLE =
  128 |   "demo Postgres not reachable from the orchestrator — set ORCH_TEST_DB_HOST/ports or run `./install.sh --demo`";
  129 | 
  130 | /** Poll `fn` until it returns a truthy value or the timeout elapses. */
  131 | export async function waitFor<T>(fn: () => Promise<T | undefined | null | false>, timeoutMs: number, label: string): Promise<T> {
  132 |   const deadline = Date.now() + timeoutMs;
  133 |   let last: unknown;
  134 |   while (Date.now() < deadline) {
  135 |     try {
  136 |       const value = await fn();
  137 |       if (value) return value as T;
  138 |     } catch (error) {
  139 |       last = error;
  140 |     }
  141 |     await new Promise((r) => setTimeout(r, 2000));
  142 |   }
  143 |   throw new Error(`timed out after ${timeoutMs}ms waiting for ${label}${last ? `: ${last}` : ""}`);
  144 | }
  145 | 
```