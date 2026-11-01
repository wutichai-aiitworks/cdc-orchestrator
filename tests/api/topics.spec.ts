import { expect, test } from "@playwright/test";
import { API, RUN_PREFIX, expectError, expectStatus, json, waitFor } from "./helpers";

test.describe.configure({ mode: "serial" });

const topic = `${RUN_PREFIX}-topic`;
let createdTopic = false;

test.afterAll(async ({ request }) => {
  if (createdTopic) await request.delete(`${API}/topics/${topic}`).catch(() => undefined);
});

test("POST /topics creates a topic (audited)", async ({ request }) => {
  const res = await request.post(`${API}/topics`, { data: { name: topic, partitions: 1, replication_factor: 1 } });
  await expectStatus(res, 200);
  createdTopic = true;
  const body = await json(res);
  expect(body).toMatchObject({ action: "topic.create", status: "success" });
  expect(body.audit_id).toMatch(/^[0-9a-f-]{36}$/);
});

test("KNOWN BUG: duplicate topic create → 409", async ({ request }) => {
  test.fail(true, "duplicate create surfaces as 502 instead of 409 — see docs/testing/api-test-plan.md");
  const res = await request.post(`${API}/topics`, { data: { name: topic, partitions: 1, replication_factor: 1 } });
  expect(res.status()).toBe(409);
});

test("GET /topics?search finds it with paging fields", async ({ request }) => {
  const res = await request.get(`${API}/topics?search=${topic}`);
  await expectStatus(res, 200);
  const body = await json(res);
  expect(body.items.map((t: any) => t.name)).toEqual([topic]);
  expect(body).toMatchObject({ total: 1, page: 1 });
});

test("GET /kafka/topics includes it", async ({ request }) => {
  const body = await json(await request.get(`${API}/kafka/topics`));
  expect(body.topics.some((t: any) => t.name === topic)).toBe(true);
});

test("POST /topics/:topic/messages produces and GET consumes it", async ({ request }) => {
  const produced = await request.post(`${API}/topics/${topic}/messages`, {
    data: { key: "k1", value: JSON.stringify({ run: RUN_PREFIX }) },
  });
  await expectStatus(produced, 200);
  expect((await json(produced)).action).toBe("topic.produce");

  const item = await waitFor(
    async () => {
      const body = await json(await request.get(`${API}/topics/${topic}/messages?limit=5`));
      return body.items.find((m: any) => m.key === "k1");
    },
    20_000,
    "produced message to be readable"
  );
  expect(JSON.parse(item.payload)).toEqual({ run: RUN_PREFIX });
});

test("GET /topics/:topic/metadata reports partitions and offsets", async ({ request }) => {
  const res = await request.get(`${API}/topics/${topic}/metadata`);
  await expectStatus(res, 200);
  const body = await json(res);
  expect(body.partition_count).toBe(1);
  expect(body.partitions[0].high_watermark).toBeGreaterThanOrEqual(1);
});

test("PUT /topics/:topic/configs alters retention", async ({ request }) => {
  const res = await request.put(`${API}/topics/${topic}/configs`, { data: { configs: { "retention.ms": "3600000" } } });
  await expectStatus(res, 200);
  expect((await json(res)).action).toBe("topic.alter_config");
  const meta = await json(await request.get(`${API}/topics/${topic}/metadata`));
  expect(meta.configs["retention.ms"]).toBe("3600000");
});

test("POST /topics rejects invalid names", async ({ request }) => {
  await expectError(await request.post(`${API}/topics`, { data: { name: "bad name!!" } }), 400, /may only contain/);
});

test("internal topics cannot be deleted", async ({ request }) => {
  await expectError(await request.delete(`${API}/topics/__consumer_offsets`), 400, /internal topics/);
});

test("DELETE /topics/:topic removes it", async ({ request }) => {
  const res = await request.delete(`${API}/topics/${topic}`);
  await expectStatus(res, 200);
  createdTopic = false;
  await waitFor(
    async () => (await json(await request.get(`${API}/topics?search=${topic}`))).total === 0,
    20_000,
    "topic to disappear from listing"
  );
});

test("KNOWN BUG: metadata for a missing topic → 404", async ({ request }) => {
  test.fail(true, "returns 200 with UNKNOWN_TOPIC_OR_PARTITION buried in `raw` — see docs/testing/api-test-plan.md");
  expect((await request.get(`${API}/topics/${RUN_PREFIX}-never-existed/metadata`)).status()).toBe(404);
});

test("KNOWN BUG: connector status for a missing connector → 404", async ({ request }) => {
  test.fail(true, "Connect 404 surfaces as 502 — see docs/testing/api-test-plan.md");
  expect((await request.get(`/connect/connectors/${RUN_PREFIX}-no-such/status`)).status()).toBe(404);
});
