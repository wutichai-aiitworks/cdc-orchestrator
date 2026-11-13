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
  sourceConfig,
} from "./helpers";

// ADR 0014 — start position for log-based capture.

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

const pgLsn = (lsn: string) => ({ mode: "custom", position: { kind: "postgres_lsn", lsn } });

async function createProfile(request: any, name: string, startPosition: unknown, extra: Record<string, unknown> = {}) {
  const res = await request.post(`${API}/profiles`, {
    data: profileBody(name, { source: sourceConfig({ start_position: startPosition, ...extra }) }),
  });
  await expectStatus(res, 200);
  const { profile } = await json(res);
  created.add(profile.id);
  return profile;
}

test.describe("start position — validation (no pipeline started)", () => {
  test("default is snapshot_then_stream and round-trips", async ({ request }) => {
    const res = await request.post(`${API}/profiles`, { data: profileBody(`${RUN_PREFIX}-sp-default`) });
    await expectStatus(res, 200);
    const { profile } = await json(res);
    created.add(profile.id);
    expect(profile.source.start_position).toEqual({ mode: "snapshot_then_stream" });
  });

  test("position kind must match the source type", async ({ request }) => {
    const res = await request.post(`${API}/profiles`, {
      data: profileBody(`${RUN_PREFIX}-sp-kind`, {
        source: sourceConfig({ start_position: { mode: "custom", position: { kind: "oracle_scn", scn: "1" } } }),
      }),
    });
    await expectError(res, 400, /does not match a PostgreSQL source/);
  });

  test("malformed LSN is refused", async ({ request }) => {
    const res = await request.post(`${API}/profiles`, {
      data: profileBody(`${RUN_PREFIX}-sp-lsn`, { source: sourceConfig({ start_position: pgLsn("zz/1") }) }),
    });
    await expectError(res, 400, /invalid PostgreSQL LSN/);
  });

  test("non-log capture modes reject a start position", async ({ request }) => {
    const res = await request.post(`${API}/profiles`, {
      data: profileBody(`${RUN_PREFIX}-sp-trigger`, {
        source: sourceConfig({ capture_mode: "trigger", start_position: { mode: "latest" } }),
      }),
    });
    await expectError(res, 400, /capture_mode=logical/);
  });

  test("position of a never-started pipeline has no committed offset", async ({ request }) => {
    const profile = await createProfile(request, `${RUN_PREFIX}-sp-pos`, { mode: "latest" });
    const res = await request.get(`${API}/profiles/${profile.id}/position`);
    await expectStatus(res, 200);
    const body = await json(res);
    expect(body.start_position).toEqual({ mode: "latest" });
    expect(body.committed).toBeNull();
  });

  test("reposition needs a source connector", async ({ request }) => {
    const profile = await createProfile(request, `${RUN_PREFIX}-sp-repo`, { mode: "latest" });
    const res = await request.post(`${API}/profiles/${profile.id}/reposition`, {
      data: { start_position: { mode: "latest" } },
    });
    await expectError(res, 400, /no source connector yet/);
  });

  test("reposition of an unknown pipeline is 404", async ({ request }) => {
    const res = await request.post(`${API}/profiles/00000000-0000-0000-0000-000000000000/reposition`, {
      data: { start_position: { mode: "latest" } },
    });
    await expectStatus(res, 404);
  });
});

test.describe("start position — live source (ORCH_TEST_E2E=true)", () => {
  test.skip(!e2eEnabled, "set ORCH_TEST_E2E=true to start pipelines on the demo source");
  test.setTimeout(120_000);

  const connectors: string[] = [];
  const slots: string[] = [];
  const prefixes: string[] = [];

  test.afterAll(async ({ request }) => {
    for (const name of connectors) {
      await request.delete(`/connect/connectors/${encodeURIComponent(name)}`).catch(() => undefined);
    }
    for (const slot of slots) {
      for (let i = 0; i < 4; i++) {
        await new Promise((r) => setTimeout(r, 3000));
        await request.post(`${API}/compare/target/exec-ddl`, {
          data: {
            target: compareConnection(db.sourcePort),
            statement: `SELECT pg_drop_replication_slot(slot_name) FROM pg_replication_slots WHERE slot_name = '${slot}' AND NOT active`,
          },
        });
      }
    }
    for (const prefix of prefixes) {
      const topics = await json(await request.get(`${API}/topics?search=${prefix}&page_size=100`)).catch(() => ({ items: [] }));
      for (const t of topics.items ?? []) {
        if (String(t.name).startsWith(`${prefix}.`)) await request.delete(`${API}/topics/${t.name}`);
      }
    }
  });

  const track = (name: string) => {
    const prefix = name.replace(/[^a-zA-Z0-9]/g, "_").toLowerCase();
    connectors.push(`cdc-${name}`, `cdc-${name}-sink`);
    slots.push(`cdc_${prefix}`);
    prefixes.push(prefix);
  };

  test("earliest_retained without a slot fails before creating connectors", async ({ request }) => {
    test.skip(!demo, DEMO_UNREACHABLE);
    const name = `${RUN_PREFIX}-sp-early`;
    track(name);
    const profile = await createProfile(request, name, { mode: "earliest_retained" });
    await expectError(await request.post(`${API}/profiles/${profile.id}/actions/start`), 400, /needs an existing replication slot/);
    const probe = await request.get(`/connect/connectors/cdc-${name}/config`);
    expect(probe.ok()).toBe(false);
  });

  test("custom LSN older than a new slot is refused", async ({ request }) => {
    test.skip(!demo, DEMO_UNREACHABLE);
    const name = `${RUN_PREFIX}-sp-old`;
    track(name);
    const profile = await createProfile(request, name, pgLsn("0/1"));
    await expectError(await request.post(`${API}/profiles/${profile.id}/actions/start`), 400, /does not exist yet/);
  });

  test("latest starts without a snapshot, and resync is refused", async ({ request }) => {
    test.skip(!demo, DEMO_UNREACHABLE);
    const name = `${RUN_PREFIX}-sp-latest`;
    track(name);
    const profile = await createProfile(request, name, { mode: "latest" });
    await expectStatus(await request.post(`${API}/profiles/${profile.id}/actions/start`), 200);
    const cfgRes = await request.get(`/connect/connectors/cdc-${name}/config`);
    const cfg = await cfgRes.json();
    const config = (cfg.config ?? cfg) as Record<string, string>;
    expect(config["snapshot.mode"]).toBe("no_data");
    expect(config["cdc.start_position"]).toBe('{"mode":"latest"}');
    await expectError(await request.post(`${API}/profiles/${profile.id}/actions/resync`), 400, /Reposition/);

    const repo = await request.post(`${API}/profiles/${profile.id}/reposition`, {
      data: { start_position: { mode: "latest" } },
    });
    await expectStatus(repo, 200);
    const body = await json(repo);
    expect(body.position.kind).toBe("postgres_lsn");
    expect(body.position.lsn).toMatch(/^[0-9A-F]+\/[0-9A-F]+$/);
  });
});
