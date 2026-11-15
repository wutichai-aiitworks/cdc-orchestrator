import { expect, test } from "@playwright/test";
import { API, expectStatus, json, RUN_PREFIX } from "./helpers";

// SSH tunnels (ADR 0020). Fixtures are `pw-` prefixed and deleted in afterAll. These tests need the
// cdc-sshtunnel gateway configured but no SSH server: they cover validation, redaction, the
// secret-per-login rule and the unpinned-tunnel guard. The full path (SSH server + Debezium through
// a forward) is the manual/e2e run described in docs/ssh-tunnel-sources.md.

const created: string[] = [];
// Not a real key: the API only checks presence; the gateway parses it at connect/test time.
const FAKE_KEY = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n";

const tunnel = (over: Record<string, unknown> = {}) => ({
  name: `${RUN_PREFIX}-ssh`,
  host: "ssh.pw-test.invalid",
  port: 22,
  username: "cdc",
  auth_method: "key",
  private_key: FAKE_KEY,
  proxy: { kind: "cloudflare_access", hostname: "ssh.pw-test.invalid", service_token_id: "id.access", service_token_secret: "pw-secret" },
  ...over,
});

test.afterAll(async ({ request }) => {
  for (const id of created) await request.delete(`${API}/ssh-tunnels/${id}`).catch(() => {});
});

test.describe("ssh tunnels", () => {
  test("capability reports the gateway", async ({ request }) => {
    const cap = await json(await request.get(`${API}/ssh-tunnels/capability`));
    test.skip(cap.configured === false, "ORCH_SSHTUNNEL_URL not set");
    expect(cap.engine).toBe("ssh");
    expect(cap.forward_ports).toMatch(/^\d+-\d+$/);
  });

  test("validation: key required, token id and secret together, known proxy kind", async ({ request }) => {
    const noKey = await request.post(`${API}/ssh-tunnels`, { data: tunnel({ private_key: "" }) });
    expect(noKey.status()).toBe(400);
    const halfToken = await request.post(`${API}/ssh-tunnels`, {
      data: tunnel({ proxy: { kind: "cloudflare_access", hostname: "h.invalid", service_token_id: "only-id" } }),
    });
    expect(halfToken.status()).toBe(400);
    const badKind = await request.post(`${API}/ssh-tunnels`, { data: tunnel({ proxy: { kind: "socks" } }) });
    expect(badKind.status()).toBe(400);
  });

  test("create redacts secrets; ******** kept for the same login, refused for another host", async ({ request }) => {
    const res = await request.post(`${API}/ssh-tunnels`, { data: tunnel() });
    await expectStatus(res, 200);
    const t = await json(res);
    created.push(t.id);
    expect(t.private_key).toBe("********");
    expect(t.proxy.service_token_secret).toBe("********");
    expect(t.host_key_fingerprint).toBe("");
    expect(t.forwards).toEqual([]);

    const listed = await json(await request.get(`${API}/ssh-tunnels`));
    const mine = listed.tunnels.find((x: any) => x.id === t.id);
    expect(mine.private_key).toBe("********");
    expect(mine.used_by).toEqual([]);

    const same = await request.put(`${API}/ssh-tunnels/${t.id}`, {
      data: tunnel({ private_key: "********", proxy: { ...tunnel().proxy, service_token_secret: "********" }, notes: "edited" }),
    });
    await expectStatus(same, 200);
    expect((await json(same)).notes).toBe("edited");

    const moved = await request.put(`${API}/ssh-tunnels/${t.id}`, {
      data: tunnel({
        host: "other.pw-test.invalid",
        private_key: "********",
        proxy: { kind: "cloudflare_access", hostname: "other.pw-test.invalid", service_token_id: "id.access", service_token_secret: "********" },
      }),
    });
    expect(moved.status()).toBe(400);
  });

  test("an unpinned tunnel is stored but not pushed to the gateway", async ({ request }) => {
    const res = await request.post(`${API}/ssh-tunnels`, { data: tunnel({ name: `${RUN_PREFIX}-ssh-unpinned` }) });
    await expectStatus(res, 200);
    const t = await json(res);
    created.push(t.id);
    const listed = await json(await request.get(`${API}/ssh-tunnels`));
    const mine = listed.tunnels.find((x: any) => x.id === t.id);
    expect(mine.status ?? null).toBeNull();
  });
});
