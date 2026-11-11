// node --test --import ./tests/unit/register-ts.mjs tests/unit/*.test.ts
import assert from "node:assert/strict"
import { test } from "node:test"

import * as r from "../../cdc-native-ui/utils/tunnelReadiness.ts"

const lanTunnel = (status: any = null) => ({
  address: "10.66.1.2/32",
  site_address: "10.66.1.1/32",
  peer_endpoint: "vpn.site.example:51820",
  allowed_ips: ["10.66.1.1/32", "192.168.10.0/24"],
  status,
})
const up = { connected: true, handshake_age_secs: 8, resolved_endpoint: "203.0.113.5:51820" }
const states = (s: r.ReadinessStep[]) => Object.fromEntries(s.map((x) => [x.key, x.state]))

test("readiness: nothing works yet — the first failing step is ours", () => {
  const s = r.tunnelReadiness(lanTunnel(), false, null)
  assert.deepEqual(states(s), { gateway: "fail", loaded: "pending", dns: "pending", handshake: "pending", forward: "pending", listen: "pending", login: "manual" })
  assert.equal(r.readinessSummary(s).blocking?.key, "gateway")
})

test("readiness: no handshake points at the site's UDP port, WireGuard and key", () => {
  const s = r.tunnelReadiness(lanTunnel({ connected: false, handshake_age_secs: null, resolved_endpoint: "203.0.113.5:51820" }), true, null)
  const hs = s.find((x) => x.key === "handshake")!
  assert.equal(hs.state, "fail")
  assert.match(hs.fix!, /UDP 51820/)
  assert.match(hs.commands!, /ufw allow 51820\/udp/)
  assert.equal(r.readinessSummary(s).blocking?.key, "handshake")
})

test("readiness: a timeout blames forwarding, a refusal proves forwarding works", () => {
  const timeout = r.tunnelReadiness(lanTunnel(up), true, { ok: false, error: "connect tcp 192.168.10.5:5432: i/o timeout", target: "192.168.10.5:5432" })
  assert.equal(states(timeout).forward, "fail")
  assert.equal(states(timeout).listen, "fail")
  const refused = r.tunnelReadiness(lanTunnel(up), true, { ok: false, error: "connection was refused", target: "192.168.10.5:5432" })
  assert.equal(states(refused).forward, "ok")
  assert.equal(states(refused).listen, "fail")
  assert.match(refused.find((x) => x.key === "listen")!.detail, /nothing listens/)
})

test("readiness: all reachable leaves only the login to confirm", () => {
  const s = r.tunnelReadiness(lanTunnel(up), true, { ok: true, target: "192.168.10.5:5432", ms: 12 })
  assert.deepEqual(states(s), { gateway: "ok", loaded: "ok", dns: "ok", handshake: "ok", forward: "ok", listen: "ok", login: "manual" })
  const sum = r.readinessSummary(s)
  assert.equal(sum.done, 6)
  assert.equal(sum.total, 7)
  assert.equal(sum.blocking?.key, "login")
})

test("readiness: database on the WireGuard host needs no forwarding; allow the gateway IP itself", () => {
  const t = { ...lanTunnel(up), allowed_ips: ["10.66.1.1/32"] }
  assert.equal(states(r.tunnelReadiness(t, true, null)).forward, "skip")
  assert.deepEqual(r.dbSourceAddress(t), { address: "10.66.1.2", masquerade: false })
  assert.match(r.dbAllowCommands("postgres", t), /host  all          cdc_user  10\.66\.1\.2\/32/)
  // With a LAN network the WireGuard host masquerades: the database sees its LAN address.
  assert.equal(r.dbSourceAddress(lanTunnel()).masquerade, true)
  assert.match(r.dbAllowCommands("mysql", lanTunnel()), /'cdc'@'<wg-host-lan-ip>'/)
})

test("handshake step shows the keys the site must run, so an old wg0.conf is easy to spot", async () => {
  const { tunnelReadiness } = await import("../../cdc-native-ui/utils/tunnelReadiness.ts")
  const t = {
    address: "10.66.1.2/32", site_address: "10.66.1.1/32", peer_endpoint: "203.0.113.5:51820", allowed_ips: ["10.66.1.1/32"],
    public_key: "GWPUB=", peer_public_key: "SITEPUB=", status: { connected: false, handshake_age_secs: null },
  }
  const step = tunnelReadiness(t, true, null).find((s) => s.key === "handshake")!
  assert.equal(step.state, "fail")
  assert.ok(step.fix!.includes('"public key: SITEPUB="') && step.fix!.includes('"peer: GWPUB="'))
  assert.ok(step.commands!.includes("sudo wg show wg0 public-key   # expect SITEPUB="))
  assert.ok(step.commands!.includes("sudo wg show wg0 peers        # expect GWPUB="))
  const ok = tunnelReadiness({ ...t, status: { connected: true, handshake_age_secs: 5 } }, true, null).find((s) => s.key === "handshake")!
  assert.equal(ok.fix, undefined)
})

test("last step names the tool check that confirms the login (SSH: File sync → Test login)", async () => {
  const { tunnelReadiness } = await import("../../cdc-native-ui/utils/tunnelReadiness.ts")
  const t = { address: "10.66.1.2/32", site_address: "10.66.1.1/32", peer_endpoint: "203.0.113.5:51820", allowed_ips: ["10.66.1.1/32"], status: { connected: true, handshake_age_secs: 5 } }
  const ssh = tunnelReadiness(t, true, null, "ssh").find((s) => s.key === "login")!
  assert.equal(ssh.title, "SSH login and folder access")
  assert.ok(ssh.detail.includes("Test login") && ssh.detail.includes("10.66.1.1"))
  assert.equal(tunnelReadiness(t, true, null, "postgres").find((s) => s.key === "login")!.title, "Database login and privileges")
})

test("rsync daemon: port 873, rsyncd.conf bound to the tunnel and limited to the gateway, File sync check named", async () => {
  const { ENGINES, dbAllowCommands, tunnelReadiness } = await import("../../cdc-native-ui/utils/tunnelReadiness.ts")
  assert.equal(ENGINES.find((e) => e.value === "rsyncd")!.port, 873)
  const t = { address: "10.66.1.2/32", site_address: "10.66.1.1/32", peer_endpoint: "203.0.113.5:51820", allowed_ips: ["10.66.1.1/32"], status: { connected: true, handshake_age_secs: 5 } }
  const c = dbAllowCommands("rsyncd", t)
  for (const want of ["address = 10.66.1.1", "hosts allow = 10.66.1.2", "hosts deny = *", "chmod 600 /etc/rsyncd.secrets", "systemctl enable --now rsync"]) assert.ok(c.includes(want), want)
  const login = tunnelReadiness(t, true, null, "rsyncd").find((s) => s.key === "login")!
  assert.equal(login.title, "rsync daemon module and login")
  assert.ok(login.detail.includes("rsync daemon") && login.detail.includes("Test connection"))
})
