import { expect, test } from "@playwright/test";
import { API, expectStatus, json } from "./helpers";

// Read-only operational endpoints: discovery, cluster, metrics, audit, logs.

test("GET /capability advertises service, version and prefix", async ({ request }) => {
  const res = await request.get(`${API}/capability`);
  await expectStatus(res, 200);
  const body = await json(res);
  expect(body.service).toBe("hyper-cdc-orchestrator");
  expect(body.api_prefix).toBe("/cdc");
  expect(body.version).toMatch(/^\d+\.\d+\.\d+/);
  expect(body.features.profiles).toBe(true);
});

test("legacy (unprefixed) and /cdc routes both serve", async ({ request }) => {
  await expectStatus(await request.get("/profiles"), 200);
  await expectStatus(await request.get(`${API}/profiles`), 200);
});

test("GET /cluster returns the Kafka cluster id", async ({ request }) => {
  const body = await json(await request.get(`${API}/cluster`));
  expect(body.kafka_cluster_id).toBeTruthy();
});

test("GET /brokers lists at least one broker, /brokers/:id resolves it", async ({ request }) => {
  const body = await json(await request.get(`${API}/brokers`));
  expect(body.items.length).toBeGreaterThan(0);
  const id = body.items[0].id;
  await expectStatus(await request.get(`${API}/brokers/${id}`), 200);
});

test("GET /metrics exposes Prometheus text", async ({ request }) => {
  const res = await request.get(`${API}/metrics`);
  await expectStatus(res, 200);
  const text = await res.text();
  expect(text).toMatch(/^# HELP cdc_/m);
  expect(text).toMatch(/^# TYPE cdc_\w+ (gauge|counter)/m);
});

test("GET /audit/events honours limit and filters", async ({ request }) => {
  const res = await request.get(`${API}/audit/events?limit=3`);
  await expectStatus(res, 200);
  const body = await json(res);
  expect(body.items.length).toBeLessThanOrEqual(3);
  expect(body.count).toBe(body.items.length);
  for (const e of body.items) expect(e).toHaveProperty("action");

  const filtered = await json(await request.get(`${API}/audit/events?outcome=success&limit=10`));
  for (const e of filtered.items) expect(e.outcome).toBe("success");
});

test("GET /pipeline-logs returns items", async ({ request }) => {
  const res = await request.get(`${API}/pipeline-logs?limit=5`);
  await expectStatus(res, 200);
  const body = await json(res);
  expect(body.items.length).toBeLessThanOrEqual(5);
});

test("GET /notifications/history returns a list", async ({ request }) => {
  const res = await request.get(`${API}/notifications/history`);
  await expectStatus(res, 200);
  expect(Array.isArray(await json(res))).toBe(true);
});

test("GET /logs lists services, /logs/loggers lists Connect loggers", async ({ request }) => {
  const services = await json(await request.get(`${API}/logs`));
  expect(Object.keys(services).length).toBeGreaterThan(0);
  const loggers = await json(await request.get(`${API}/logs/loggers`));
  expect(loggers.root?.level).toBeTruthy();
});
