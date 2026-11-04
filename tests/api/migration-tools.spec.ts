import { expect, test } from "@playwright/test";
import { API, expectStatus, json } from "./helpers";

// One-shot migration tools share one contract (pgcopydb, pgloader, redis-shake, mongo copy, pg_dump).
// Read-only: no profiles are created and no jobs are run.

const TOOLS = ["pgcopy", "pgloader", "redisshake", "mongocopy", "pgdump"] as const;

test("GET /migration-tools/discover lists every tool with resolvable paths", async ({ request }) => {
  const res = await request.get(`${API}/migration-tools/discover`);
  await expectStatus(res, 200);
  const body = await json(res);
  expect(body.contract_version).toBeTruthy();
  const ids = body.tools.map((t: any) => t.id);
  for (const tool of TOOLS) expect(ids).toContain(tool);

  for (const t of body.tools) {
    expect(t.contract_path).toBe(`${API}/${t.id}/contract`);
    expect(t.discover_path).toBe(`${API}/${t.id}/discover`);
    await expectStatus(await request.get(t.contract_path), 200);
  }
});

for (const tool of TOOLS) {
  test.describe(`${tool}`, () => {
    for (const path of ["capability", "contract", "discover"]) {
      test(`GET /${tool}/${path}`, async ({ request }) => {
        const res = await request.get(`${API}/${tool}/${path}`);
        await expectStatus(res, 200);
        expect(typeof (await json(res))).toBe("object");
      });
    }

    test(`GET /${tool}/profiles returns a profile list`, async ({ request }) => {
      const res = await request.get(`${API}/${tool}/profiles`);
      await expectStatus(res, 200);
      expect(Array.isArray((await json(res)).profiles)).toBe(true);
    });

    test(`GET /${tool}/jobs responds`, async ({ request }) => {
      const res = await request.get(`${API}/${tool}/jobs`);
      await expectStatus(res, 200);
      await json(res);
    });

    test(`GET /${tool}/profiles/:unknown → 404`, async ({ request }) => {
      const res = await request.get(`${API}/${tool}/profiles/00000000-0000-0000-0000-000000000000`);
      expect(res.status()).toBe(404);
    });

    test(`GET /migration-tools/${tool}/contract matches /${tool}/contract`, async ({ request }) => {
      const a = await json(await request.get(`${API}/migration-tools/${tool}/contract`));
      const b = await json(await request.get(`${API}/${tool}/contract`));
      const strip = (x: any) => JSON.stringify({ ...x, timestamp: undefined });
      expect(strip(a)).toBe(strip(b));
    });
  });
}
