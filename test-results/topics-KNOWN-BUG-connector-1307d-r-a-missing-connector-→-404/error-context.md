# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: topics.spec.ts >> KNOWN BUG: connector status for a missing connector → 404
- Location: tests/api/topics.spec.ts:99:5

# Error details

```
Error: expect(received).toBe(expected) // Object.is equality

Expected: 404
Received: 502
```

# Test source

```ts
  1   | import { expect, test } from "@playwright/test";
  2   | import { API, RUN_PREFIX, expectError, expectStatus, json, waitFor } from "./helpers";
  3   | 
  4   | test.describe.configure({ mode: "serial" });
  5   | 
  6   | const topic = `${RUN_PREFIX}-topic`;
  7   | let createdTopic = false;
  8   | 
  9   | test.afterAll(async ({ request }) => {
  10  |   if (createdTopic) await request.delete(`${API}/topics/${topic}`).catch(() => undefined);
  11  | });
  12  | 
  13  | test("POST /topics creates a topic (audited)", async ({ request }) => {
  14  |   const res = await request.post(`${API}/topics`, { data: { name: topic, partitions: 1, replication_factor: 1 } });
  15  |   await expectStatus(res, 200);
  16  |   createdTopic = true;
  17  |   const body = await json(res);
  18  |   expect(body).toMatchObject({ action: "topic.create", status: "success" });
  19  |   expect(body.audit_id).toMatch(/^[0-9a-f-]{36}$/);
  20  | });
  21  | 
  22  | test("KNOWN BUG: duplicate topic create → 409", async ({ request }) => {
  23  |   test.fail(true, "duplicate create surfaces as 502 instead of 409 — see docs/testing/api-test-plan.md");
  24  |   const res = await request.post(`${API}/topics`, { data: { name: topic, partitions: 1, replication_factor: 1 } });
  25  |   expect(res.status()).toBe(409);
  26  | });
  27  | 
  28  | test("GET /topics?search finds it with paging fields", async ({ request }) => {
  29  |   const res = await request.get(`${API}/topics?search=${topic}`);
  30  |   await expectStatus(res, 200);
  31  |   const body = await json(res);
  32  |   expect(body.items.map((t: any) => t.name)).toEqual([topic]);
  33  |   expect(body).toMatchObject({ total: 1, page: 1 });
  34  | });
  35  | 
  36  | test("GET /kafka/topics includes it", async ({ request }) => {
  37  |   const body = await json(await request.get(`${API}/kafka/topics`));
  38  |   expect(body.topics.some((t: any) => t.name === topic)).toBe(true);
  39  | });
  40  | 
  41  | test("POST /topics/:topic/messages produces and GET consumes it", async ({ request }) => {
  42  |   const produced = await request.post(`${API}/topics/${topic}/messages`, {
  43  |     data: { key: "k1", value: JSON.stringify({ run: RUN_PREFIX }) },
  44  |   });
  45  |   await expectStatus(produced, 200);
  46  |   expect((await json(produced)).action).toBe("topic.produce");
  47  | 
  48  |   const item = await waitFor(
  49  |     async () => {
  50  |       const body = await json(await request.get(`${API}/topics/${topic}/messages?limit=5`));
  51  |       return body.items.find((m: any) => m.key === "k1");
  52  |     },
  53  |     20_000,
  54  |     "produced message to be readable"
  55  |   );
  56  |   expect(JSON.parse(item.payload)).toEqual({ run: RUN_PREFIX });
  57  | });
  58  | 
  59  | test("GET /topics/:topic/metadata reports partitions and offsets", async ({ request }) => {
  60  |   const res = await request.get(`${API}/topics/${topic}/metadata`);
  61  |   await expectStatus(res, 200);
  62  |   const body = await json(res);
  63  |   expect(body.partition_count).toBe(1);
  64  |   expect(body.partitions[0].high_watermark).toBeGreaterThanOrEqual(1);
  65  | });
  66  | 
  67  | test("PUT /topics/:topic/configs alters retention", async ({ request }) => {
  68  |   const res = await request.put(`${API}/topics/${topic}/configs`, { data: { configs: { "retention.ms": "3600000" } } });
  69  |   await expectStatus(res, 200);
  70  |   expect((await json(res)).action).toBe("topic.alter_config");
  71  |   const meta = await json(await request.get(`${API}/topics/${topic}/metadata`));
  72  |   expect(meta.configs["retention.ms"]).toBe("3600000");
  73  | });
  74  | 
  75  | test("POST /topics rejects invalid names", async ({ request }) => {
  76  |   await expectError(await request.post(`${API}/topics`, { data: { name: "bad name!!" } }), 400, /may only contain/);
  77  | });
  78  | 
  79  | test("internal topics cannot be deleted", async ({ request }) => {
  80  |   await expectError(await request.delete(`${API}/topics/__consumer_offsets`), 400, /internal topics/);
  81  | });
  82  | 
  83  | test("DELETE /topics/:topic removes it", async ({ request }) => {
  84  |   const res = await request.delete(`${API}/topics/${topic}`);
  85  |   await expectStatus(res, 200);
  86  |   createdTopic = false;
  87  |   await waitFor(
  88  |     async () => (await json(await request.get(`${API}/topics?search=${topic}`))).total === 0,
  89  |     20_000,
  90  |     "topic to disappear from listing"
  91  |   );
  92  | });
  93  | 
  94  | test("KNOWN BUG: metadata for a missing topic → 404", async ({ request }) => {
  95  |   test.fail(true, "returns 200 with UNKNOWN_TOPIC_OR_PARTITION buried in `raw` — see docs/testing/api-test-plan.md");
  96  |   expect((await request.get(`${API}/topics/${RUN_PREFIX}-never-existed/metadata`)).status()).toBe(404);
  97  | });
  98  | 
  99  | test("KNOWN BUG: connector status for a missing connector → 404", async ({ request }) => {
  100 |   test.fail(true, "Connect 404 surfaces as 502 — see docs/testing/api-test-plan.md");
> 101 |   expect((await request.get(`/connect/connectors/${RUN_PREFIX}-no-such/status`)).status()).toBe(404);
      |                                                                                            ^ Error: expect(received).toBe(expected) // Object.is equality
  102 | });
  103 | 
```