// node --test --import ./tests/unit/register-ts.mjs tests/unit/*.test.ts
import assert from "node:assert/strict"
import { test } from "node:test"

import * as wg from "../../cdc-native-ui/utils/wireguardView.ts"

test("wireguard: each new tunnel gets its own /24 (gateway .2, site .1)", () => {
  assert.deepEqual(wg.suggestAddresses([]), { address: "10.66.1.2/32", site_address: "10.66.1.1/32" })
  assert.deepEqual(wg.suggestAddresses([{ address: "10.66.1.2/32", site_address: "10.66.1.1/32" }, { address: "10.66.3.2/32" }]), {
    address: "10.66.2.2/32",
    site_address: "10.66.2.1/32",
  })
  const d = wg.emptyDraft([])
  assert.deepEqual(d.allowed_ips, ["10.66.1.1/32"])
  assert.equal(d.generate_site_keys, true)
})

test("wireguard: draft problems are explained before saving", () => {
  const d = wg.emptyDraft([])
  const p = wg.draftProblems(d, true)
  assert.ok(p.some((x) => x.startsWith("Give the tunnel a name")))
  assert.ok(p.some((x) => x.startsWith("Site endpoint must be host:port")))
  assert.ok(!p.some((x) => x.includes("public key")), "generated site keys need no pasted key")
  const ok = { ...d, name: "Source site", peer_endpoint: "vpn.source.example:51820" }
  assert.deepEqual(wg.draftProblems(ok, true), [])
  assert.ok(wg.draftProblems({ ...ok, generate_site_keys: false }, true).some((x) => x.includes("public key")))
  assert.ok(wg.draftProblems({ ...ok, allowed_ips: ["192.168.10.0"] }, true).some((x) => x.includes("not a network")))
})

test("wireguard: status reads like a sentence", () => {
  assert.deepEqual(wg.tunnelStatus({ connected: true, handshake_age_secs: 12 }), { tone: "ok", text: "Connected · handshake 12 s ago" })
  assert.deepEqual(wg.tunnelStatus({ connected: false, handshake_age_secs: 7200 }), { tone: "err", text: "Disconnected · last handshake 2 h ago" })
  assert.deepEqual(wg.tunnelStatus({ connected: false, handshake_age_secs: null }), { tone: "warn", text: "No handshake yet" })
  assert.equal(wg.tunnelStatus(null, false).text, "Gateway unreachable")
  assert.equal(wg.fmtBytes(1536), "1.5 KB")
})

test("wireguard: requests keep saved secrets and only generate on create", () => {
  const d = { ...wg.emptyDraft([]), name: "S", peer_endpoint: "h:51820", preshared_key: "********" }
  const create = wg.toRequest(d, true)
  assert.equal(create.generate_site_keys, true)
  assert.equal(create.peer_public_key, "")
  const update = wg.toRequest({ ...d, peer_public_key: "PUB" }, false)
  assert.equal(update.generate_site_keys, false)
  assert.equal(update.generate_preshared_key, false)
  assert.equal(update.preshared_key, "********")
})

test("site setup: placeholders are detected; manual steps carry no secrets and install nothing by themselves", async () => {
  const { configHasPlaceholders, siteManualSteps } = await import("../../cdc-native-ui/utils/wireguardView.ts")
  assert.equal(configHasPlaceholders("[Interface]\nPrivateKey = abc=\n"), false)
  assert.equal(configHasPlaceholders("PrivateKey = <the site's private key: wg genkey>\n"), true)
  assert.equal(configHasPlaceholders("PresharedKey = <the preshared key shown when it was generated>\n"), true)
  const steps = siteManualSteps(51820)
  const all = steps.map((s) => s.commands).join("\n")
  assert.equal(steps.length, 4)
  assert.ok(all.includes("install -m 600 /tmp/wg0.conf /etc/wireguard/wg0.conf"))
  assert.ok(all.includes("systemctl enable --now wg-quick@wg0") && all.includes("UDP4-LISTEN:51820"))
  assert.ok(!/PrivateKey|PresharedKey/.test(all), "no secrets in the steps")
})
