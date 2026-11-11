// node --test --import ./tests/unit/register-ts.mjs tests/unit/*.test.ts
import assert from "node:assert/strict"
import { test } from "node:test"

import * as prep from "../../cdc-native-ui/utils/sitePreparation.ts"

test("site prep: steps follow the platform and the chosen UDP port", () => {
  const s = prep.preparationSteps({ port: 51999, os: "rhel", firewall: "firewalld", edge: "aws" })
  assert.deepEqual(s.map((x) => x.key), ["host", "install", "hostfw", "edge", "keys", "udptest", "db"])
  const by = Object.fromEntries(s.map((x) => [x.key, x]))
  assert.match(by.install.commands!, /dnf install -y wireguard-tools socat/)
  assert.match(by.hostfw.commands!, /--add-port=51999\/udp/)
  assert.match(by.edge.commands!, /FromPort=51999,ToPort=51999/)
  assert.match(by.udptest.commands!, /socat -v UDP4-LISTEN:51999,fork PIPE/)
  assert.equal(by.keys.optional, true)
  const ubuntu = prep.preparationSteps(prep.DEFAULT_PREP)
  assert.match(ubuntu.find((x) => x.key === "hostfw")!.commands!, /ufw allow 51820\/udp/)
  assert.match(ubuntu.find((x) => x.key === "edge")!.commands!, /Port forward  UDP 51820/)
})

test("site prep: the handoff checklist has the information, every step and its commands", () => {
  const md = prep.siteChecklistMarkdown({ port: 51820, os: "debian", firewall: "ufw", edge: "router" }, "Bangkok DC")
  assert.match(md, /^# Hyper CDC WireGuard — site preparation \(Bangkok DC\)/)
  assert.equal((md.match(/^- \[ \] /gm) || []).length, prep.INFO_TO_COLLECT.length)
  assert.equal((md.match(/^### /gm) || []).length, 7)
  assert.match(md, /```bash\nsudo apt update && sudo apt install -y wireguard wireguard-tools socat\n```/)
})

test("site prep: UDP test results read clearly", () => {
  assert.deepEqual(prep.udpTestSummary({ ok: true, echoed: true, ms: 41, resolved: "203.0.113.10:51820", endpoint: "vpn:51820" }), {
    tone: "ok",
    text: "UDP port open: the echo came back from 203.0.113.10:51820 in 41 ms.",
  })
  assert.equal(prep.udpTestSummary({ ok: false, endpoint: "vpn:51820", resolved: "203.0.113.10:51820" }).tone, "warn")
  assert.equal(prep.udpTestSummary({ ok: false, error: "cannot resolve vpn", endpoint: "vpn:51820" }).tone, "err")
})
