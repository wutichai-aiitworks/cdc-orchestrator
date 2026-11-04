import { createHash } from "node:crypto";
import { expect, test } from "@playwright/test";
import {
  API,
  compareConnection,
  db,
  e2eEnabled,
  expectError,
  expectStatus,
  json,
  RUN_PREFIX,
  waitFor,
} from "./helpers";

// Postgres remote dump (cdc-pgdump sidecar). Profiles are `pw-` prefixed and deleted in afterAll.
// Jobs that dump real data (and download the file) run only with ORCH_TEST_E2E=true; they read
// the demo source and never write to the target.

const created: string[] = [];
const jobs: string[] = [];

const source = () => compareConnection(db.sourcePort);
// "No secret in the response" only means something when the password is not also the user or
// database name (the demo default is demo/demo/demo).
const passwordIsDistinct =
  db.password.length >= 6 && ![db.username, db.database, db.schema].includes(db.password);
const target = () => compareConnection(db.targetPort);


test.afterAll(async ({ request }) => {
  for (const id of jobs) await request.delete(`${API}/pgdump/jobs/${id}`).catch(() => {});
  for (const id of created) await request.delete(`${API}/pgdump/profiles/${id}`).catch(() => {});
});

test.describe("pgdump profiles", () => {
  test("create redacts secrets; update with ******** keeps the stored password", async ({ request }) => {
    const res = await request.post(`${API}/pgdump/profiles`, {
      data: { name: `${RUN_PREFIX}-dump`, mode: "dump", source: source(), options: { schemas: [db.schema] } },
    });
    await expectStatus(res, 200);
    const profile = await json(res);
    created.push(profile.id);
    expect(profile.mode).toBe("dump");
    expect(profile.source.password).toBe("********");
    expect(profile.target ?? null).toBeNull();
    expect(profile.options.schemas).toEqual([db.schema]);

    const listed = await json(await request.get(`${API}/pgdump/profiles`));
    const mine = listed.profiles.find((p: any) => p.id === profile.id);
    expect(mine.source.password).toBe("********");

    // Round-trip the redacted form (what the editor sends) and switch to dump_restore.
    const upd = await request.put(`${API}/pgdump/profiles/${profile.id}`, {
      data: { ...profile, mode: "dump_restore", target: target() },
    });
    await expectStatus(upd, 200);
    const updated = await json(upd);
    expect(updated.mode).toBe("dump_restore");
    expect(updated.target.host).toBe(db.host);
    expect(updated.target.password).toBe("********");
  });

  test("validation rejects bad mode, non-Postgres, missing target, non-custom restore", async ({ request }) => {
    const base = { name: `${RUN_PREFIX}-bad`, source: source() };
    await expectError(await request.post(`${API}/pgdump/profiles`, { data: { ...base, mode: "restore" } }), 400, /mode/);
    await expectError(
      await request.post(`${API}/pgdump/profiles`, { data: { ...base, source: { ...source(), type: "mysql" } } }),
      400,
      /PostgreSQL/,
    );
    await expectError(await request.post(`${API}/pgdump/profiles`, { data: { ...base, mode: "dump_restore" } }), 400, /target/);
    await expectError(
      await request.post(`${API}/pgdump/profiles`, {
        data: { ...base, mode: "dump_restore", target: target(), options: { format: "plain" } },
      }),
      400,
      /custom/,
    );
    await expectError(await request.post(`${API}/pgdump/profiles`, { data: { ...base, name: " " } }), 400, /name/);
  });

  test("SFTP destinations: validated, secrets redacted, kept across a redacted round-trip", async ({ request }) => {
    const dest = {
      name: "backup",
      host: "sftp.example.invalid",
      port: "22",
      username: "dumps",
      auth: "key",
      private_key: "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n",
      passphrase: "kp",
      remote_dir: "/srv/dumps",
      host_key_fingerprint: "SHA256:1ZCE5EQq47bGsZKhyBx+wXMKnh+zKwAu0xnnPyEYrPE",
    };
    const base = { name: `${RUN_PREFIX}-sftp`, mode: "dump", source: source() };
    await expectError(
      await request.post(`${API}/pgdump/profiles`, { data: { ...base, destinations: [{ ...dest, private_key: "" }] } }),
      400,
      /private_key/,
    );
    await expectError(
      await request.post(`${API}/pgdump/profiles`, { data: { ...base, destinations: [{ ...dest, remote_dir: "" }] } }),
      400,
      /remote_dir/,
    );
    await expectError(
      await request.post(`${API}/pgdump/profiles`, { data: { ...base, destinations: [{ ...dest, host_key_fingerprint: "MD5:aa" }] } }),
      400,
      /SHA256/,
    );

    const res = await request.post(`${API}/pgdump/profiles`, {
      data: { ...base, destinations: [dest, { ...dest, name: "pw", auth: "password", password: "pw-1", private_key: "", passphrase: "" }] },
    });
    await expectStatus(res, 200);
    const profile = await json(res);
    created.push(profile.id);
    expect(JSON.stringify(profile)).not.toContain("BEGIN OPENSSH");
    expect(profile.destinations.map((d: any) => [d.private_key, d.passphrase, d.password])).toEqual([
      ["********", "********", ""],
      ["", "", "********"],
    ]);

    // PUT the redacted form back: validation must still pass because stored secrets are restored.
    await expectStatus(await request.put(`${API}/pgdump/profiles/${profile.id}`, { data: profile }), 200);
  });

  test("POST /pgdump/validate checks connection fields", async ({ request }) => {
    const ok = await json(await request.post(`${API}/pgdump/validate`, { data: { connection: source() } }));
    expect(ok.valid).toBe(true);
    const bad = await json(
      await request.post(`${API}/pgdump/validate`, { data: { connection: { ...source(), database: "" } } }),
    );
    expect(bad.valid).toBe(false);
  });

  test("test-connection never sends a saved password to a changed login", async ({ request }) => {
    const res = await request.post(`${API}/pgdump/profiles`, {
      data: { name: `${RUN_PREFIX}-login`, mode: "dump", source: source() },
    });
    await expectStatus(res, 200);
    const profile = await json(res);
    created.push(profile.id);
    for (const moved of [{ host: "elsewhere.invalid" }, { port: "6543" }, { username: "someone_else" }]) {
      await expectError(
        await request.post(`${API}/pgdump/test-connection`, {
          data: { connection: { ...profile.source, ...moved }, profile_id: profile.id },
        }),
        400,
        /enter the source password again/,
      );
      // Same rule when saving: a moved login cannot inherit the stored password.
      await expectError(
        await request.put(`${API}/pgdump/profiles/${profile.id}`, {
          data: { ...profile, source: { ...profile.source, ...moved } },
        }),
        400,
        /enter the source password again/,
      );
    }
    await expectError(
      await request.post(`${API}/pgdump/test-connection`, { data: { connection: source(), side: "middle" } }),
      400,
      /side must be source or target/,
    );
  });

  test("unknown job → 404 for status, file, delete", async ({ request }) => {
    const id = "00000000-0000-0000-0000-000000000000";
    expect((await request.get(`${API}/pgdump/jobs/${id}`)).status()).toBe(404);
    expect((await request.get(`${API}/pgdump/jobs/${id}/file`)).status()).toBe(404);
    expect((await request.delete(`${API}/pgdump/jobs/${id}`)).status()).toBe(404);
  });
});

test.describe("pgdump jobs (e2e)", () => {
  test.skip(!e2eEnabled, "set ORCH_TEST_E2E=true to dump the demo source");

  test("test-connection describes the demo source from the sidecar", async ({ request }) => {
    const res = await request.post(`${API}/pgdump/profiles`, {
      data: { name: `${RUN_PREFIX}-probe`, mode: "dump", source: source() },
    });
    await expectStatus(res, 200);
    const profile = await json(res);
    created.push(profile.id);
    // The editor sends ******** for a saved password; the orchestrator fills it in.
    const ok = await json(
      await request.post(`${API}/pgdump/test-connection`, { data: { connection: profile.source, profile_id: profile.id } }),
    );
    expect(ok.ok, JSON.stringify(ok)).toBe(true);
    expect(ok.server_major).toBeGreaterThanOrEqual(10);
    expect(typeof ok.compatible).toBe("boolean");
    expect(ok.schemas.map((s: any) => s.name)).toContain(db.schema);
    if (passwordIsDistinct) expect(JSON.stringify(ok)).not.toContain(db.password);

    const wrong = await json(
      await request.post(`${API}/pgdump/test-connection`, { data: { connection: { ...source(), password: "definitely-wrong" } } }),
    );
    expect(wrong.ok).toBe(false);
    expect(wrong.error).toBeTruthy();
  });

  test("dump the demo schema, download it, verify sha256, delete it", async ({ request }) => {
    const res = await request.post(`${API}/pgdump/profiles`, {
      data: { name: `${RUN_PREFIX}-e2e`, mode: "dump", source: source(), options: { schemas: [db.schema], no_owner: true } },
    });
    await expectStatus(res, 200);
    const profile = await json(res);
    created.push(profile.id);

    const run = await request.post(`${API}/pgdump/profiles/${profile.id}/run`);
    await expectStatus(run, 200);
    const started = await json(run);
    jobs.push(started.id);

    // A second run of the same profile while the first is active is refused by the sidecar.
    const second = await request.post(`${API}/pgdump/profiles/${profile.id}/run`);
    if (second.ok()) jobs.push((await json(second)).id);
    else await expectError(second, 400, /already running/);

    const done = await waitFor(
      async () => {
        const job = await json(await request.get(`${API}/pgdump/jobs/${started.id}`));
        return ["starting", "running"].includes(job.status) ? null : job;
      },
      120_000,
      "pgdump job to finish",
    );
    expect(done.status, JSON.stringify(done)).toBe("succeeded");
    expect(done.tables_dumped).toBeGreaterThan(0);
    expect(done.artifact.bytes).toBeGreaterThan(0);
    // Job status (argv, source, logs path) never carries the password value.
    const status = JSON.stringify(done);
    if (passwordIsDistinct) {
      expect(status).not.toContain(db.password);
      expect(status).not.toContain(JSON.stringify(db.password).slice(1, -1));
    }

    const file = await request.get(`${API}/pgdump/jobs/${started.id}/file`);
    await expectStatus(file, 200);
    const bytes = await file.body();
    expect(bytes.length).toBe(done.artifact.bytes);
    expect(file.headers()["x-dump-sha256"]).toBe(done.artifact.sha256);
    expect(createHash("sha256").update(bytes).digest("hex")).toBe(done.artifact.sha256);
    expect(bytes.subarray(0, 5).toString()).toBe("PGDMP"); // custom-format archive magic

    const logs = await json(await request.get(`${API}/pgdump/jobs/${started.id}/logs`));
    expect(logs.log).toContain("dumping contents of table");

    await expectStatus(await request.delete(`${API}/pgdump/jobs/${started.id}`), 200);
    expect((await request.get(`${API}/pgdump/jobs/${started.id}/file`)).status()).toBe(404);
  });
});
