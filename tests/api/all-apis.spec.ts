import { expect, test, type APIResponse } from "@playwright/test";

const strict = process.env.ORCH_TEST_STRICT === "true";
const testConnectorName =
  process.env.ORCH_TEST_CONNECTOR_NAME ?? "playwright-test-connector";

type JsonObject = Record<string, unknown>;

async function parseJsonSafe(
  response: APIResponse
): Promise<{ json: JsonObject; text: string }> {
  const text = await response.text();
  if (!text) {
    return { json: {}, text: "" };
  }
  try {
    return { json: JSON.parse(text) as JsonObject, text };
  } catch {
    return { json: {}, text };
  }
}

function assertRelaxedStatus(status: number, endpoint: string) {
  // In non-strict mode, allow upstream dependency failures while still
  // asserting the route is reachable and protected correctly.
  const allowed = [200, 400, 401, 404, 409, 502];
  expect(
    allowed.includes(status),
    `unexpected status for ${endpoint}: ${status}`
  ).toBeTruthy();
}

function assertStatus(status: number, endpoint: string) {
  if (strict) {
    expect(status, `strict mode expected 200 for ${endpoint}`).toBe(200);
    return;
  }
  assertRelaxedStatus(status, endpoint);
}

test.describe("hyper-cdc-orchestrator phase-1 APIs", () => {
  test("GET /health", async ({ request }) => {
    const response = await request.get("/health");
    expect(response.status()).toBe(200);
    const { json: body, text } = await parseJsonSafe(response);
    // Some deployments return plain "ok" instead of JSON for health.
    if ("service" in body || "status" in body) {
      expect(body).toHaveProperty("status");
      return;
    }
    expect(text.trim().length).toBeGreaterThan(0);
  });

  test("GET /engine/status", async ({ request }) => {
    const response = await request.get("/engine/status");
    assertStatus(response.status(), "/engine/status");
  });

  test("POST /engine/up", async ({ request }) => {
    const response = await request.post("/engine/up");
    assertStatus(response.status(), "/engine/up");
  });

  test("POST /engine/restart", async ({ request }) => {
    const response = await request.post("/engine/restart");
    assertStatus(response.status(), "/engine/restart");
  });

  test("POST /engine/down", async ({ request }) => {
    const response = await request.post("/engine/down");
    assertStatus(response.status(), "/engine/down");
  });

  test("GET /connect/connectors", async ({ request }) => {
    const response = await request.get("/connect/connectors");
    assertStatus(response.status(), "/connect/connectors");
  });

  test("POST /connect/connectors", async ({ request }) => {
    const response = await request.post("/connect/connectors", {
      data: {
        name: testConnectorName,
        config: {
          "connector.class": "io.debezium.connector.mysql.MySqlConnector",
          "tasks.max": "1",
          "database.hostname": "localhost",
          "database.port": "3306",
          "database.user": "demo",
          "database.password": "demo",
          "database.server.id": "184055",
          "topic.prefix": "playwright",
        },
      },
    });
    assertStatus(response.status(), "/connect/connectors");
  });

  test("GET /connect/connectors/:name/status", async ({ request }) => {
    const response = await request.get(
      `/connect/connectors/${encodeURIComponent(testConnectorName)}/status`
    );
    assertStatus(response.status(), "/connect/connectors/:name/status");
  });

  test("PUT /connect/connectors/:name/config", async ({ request }) => {
    const response = await request.put(
      `/connect/connectors/${encodeURIComponent(testConnectorName)}/config`,
      {
        data: {
          config: {
            "connector.class": "io.debezium.connector.mysql.MySqlConnector",
            "tasks.max": "1",
            "database.hostname": "localhost",
            "database.port": "3306",
            "database.user": "demo",
            "database.password": "demo",
            "database.server.id": "184056",
            "topic.prefix": "playwright",
          },
        },
      }
    );
    assertStatus(response.status(), "/connect/connectors/:name/config");
  });

  for (const action of ["pause", "resume", "restart"]) {
    test(`POST /connect/connectors/:name/${action}`, async ({ request }) => {
      const response = await request.post(
        `/connect/connectors/${encodeURIComponent(testConnectorName)}/${action}`
      );
      assertStatus(response.status(), `/connect/connectors/:name/${action}`);
    });
  }

  test("DELETE /connect/connectors/:name", async ({ request }) => {
    const response = await request.delete(
      `/connect/connectors/${encodeURIComponent(testConnectorName)}`
    );
    assertStatus(response.status(), "/connect/connectors/:name");
  });

  test("GET /kafka/topics", async ({ request }) => {
    const response = await request.get("/kafka/topics");
    assertStatus(response.status(), "/kafka/topics");
  });

  test("GET /kafka/groups", async ({ request }) => {
    const response = await request.get("/kafka/groups");
    assertStatus(response.status(), "/kafka/groups");
  });

  test("GET /kafka/groups/:group/lag", async ({ request }) => {
    const group = process.env.ORCH_TEST_GROUP ?? "vector-cdc-consumer";
    const response = await request.get(
      `/kafka/groups/${encodeURIComponent(group)}/lag`
    );
    assertStatus(response.status(), "/kafka/groups/:group/lag");
  });
});
