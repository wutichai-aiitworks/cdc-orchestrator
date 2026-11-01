import { expect, type APIRequestContext, type APIResponse } from "@playwright/test";

/**
 * Shared config for the strict API suites.
 *
 * Every fixture a suite creates is prefixed with `RUN_PREFIX` and deleted in
 * afterAll — the stack under test may also hold real customer profiles, so
 * suites must never read-modify or delete anything they did not create.
 *
 * Env:
 *   ORCH_TEST_BASE_URL        orchestrator (default http://localhost:9788)
 *   ORCH_TEST_TOKEN           Bearer token when ORCH_API_TOKEN is set
 *   ORCH_TEST_DB_HOST         host the orchestrator/jdbc-admin use to reach the demo DBs
 *                             (default host.docker.internal — the demo Postgres containers
 *                             sit on a different compose network than the orchestrator)
 *   ORCH_TEST_SOURCE_PORT     demo source port (default 25432)
 *   ORCH_TEST_TARGET_PORT     demo target port (default 25433)
 *   ORCH_TEST_DB_NAME / _USER / _PASSWORD   (default demo / demo / demo)
 *   ORCH_TEST_SCHEMA          demo schema (default bookings)
 *   ORCH_TEST_E2E=true        enable tests that start pipelines and write to the target
 */

export const API = "/cdc";

export const RUN_PREFIX = `pw-${Date.now().toString(36)}`;

const env = (key: string, fallback: string) => process.env[key]?.trim() || fallback;

export const db = {
  host: env("ORCH_TEST_DB_HOST", "host.docker.internal"),
  sourcePort: env("ORCH_TEST_SOURCE_PORT", "25432"),
  targetPort: env("ORCH_TEST_TARGET_PORT", "25433"),
  database: env("ORCH_TEST_DB_NAME", "demo"),
  username: env("ORCH_TEST_DB_USER", "demo"),
  password: env("ORCH_TEST_DB_PASSWORD", "demo"),
  schema: env("ORCH_TEST_SCHEMA", "bookings"),
  /** Small table (10 rows in the demo dataset) used for previews and replication. */
  smallTable: env("ORCH_TEST_SMALL_TABLE", "airplanes_data"),
};

export const e2eEnabled = process.env.ORCH_TEST_E2E === "true";

/** Connection map in the CDC-profile shape (`source.connection` / `target.connection`). */
export function profileConnection(port: string, password = db.password) {
  return {
    host: db.host,
    port,
    database: db.database,
    username: db.username,
    password,
  };
}

/** Connection in the schema-compare shape (`SchemaCompareConn`). */
export function compareConnection(port: string) {
  return { type: "postgres", ...profileConnection(port) };
}

export function sourceConfig(overrides: Record<string, unknown> = {}) {
  return {
    source_type: "postgres",
    connection: profileConnection(db.sourcePort),
    capture_mode: "logical",
    schema_mode: "specific_schemas",
    table_mode: "specific_tables",
    schemas: [db.schema],
    tables: [`${db.schema}.${db.smallTable}`],
    ...overrides,
  };
}

export function targetConfig(overrides: Record<string, unknown> = {}) {
  return {
    name: `${RUN_PREFIX}-target`,
    target_type: "postgres",
    connection: profileConnection(db.targetPort),
    schemas: [],
    ...overrides,
  };
}

export function profileBody(name: string, overrides: Record<string, unknown> = {}) {
  return {
    name,
    autostart: false,
    pattern: "source_to_target",
    source: sourceConfig(),
    target: targetConfig(),
    flow: { transforms: [], schedule: { delivery_mode: "realtime" }, mappings: [] },
    ...overrides,
  };
}

export async function json<T = any>(response: APIResponse): Promise<T> {
  const text = await response.text();
  try {
    return JSON.parse(text) as T;
  } catch {
    throw new Error(`expected JSON from ${response.url()} [${response.status()}], got: ${text.slice(0, 300)}`);
  }
}

/** Assert an exact status and include the body in the failure message. */
export async function expectStatus(response: APIResponse, status: number) {
  if (response.status() !== status) {
    const body = (await response.text()).slice(0, 500);
    expect(response.status(), `${response.url()} → ${body}`).toBe(status);
  }
}

/** Assert the orchestrator's error envelope `{ "error": "..." }`. */
export async function expectError(response: APIResponse, status: number, match?: RegExp) {
  await expectStatus(response, status);
  const body = await json<{ error?: string }>(response);
  expect(typeof body.error, "error envelope").toBe("string");
  if (match) expect(body.error).toMatch(match);
}

/** True when the orchestrator can reach the demo source DB (gates DB-backed tests). */
export async function demoSourceReachable(request: APIRequestContext): Promise<boolean> {
  const res = await request.post(`${API}/profiles/validate/source`, { data: sourceConfig() });
  if (!res.ok()) return false;
  const body = await res.json().catch(() => ({}));
  return body?.valid === true;
}

export const DEMO_UNREACHABLE =
  "demo Postgres not reachable from the orchestrator — set ORCH_TEST_DB_HOST/ports or run `./install.sh --demo`";

/** Poll `fn` until it returns a truthy value or the timeout elapses. */
export async function waitFor<T>(fn: () => Promise<T | undefined | null | false>, timeoutMs: number, label: string): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  let last: unknown;
  while (Date.now() < deadline) {
    try {
      const value = await fn();
      if (value) return value as T;
    } catch (error) {
      last = error;
    }
    await new Promise((r) => setTimeout(r, 2000));
  }
  throw new Error(`timed out after ${timeoutMs}ms waiting for ${label}${last ? `: ${last}` : ""}`);
}
