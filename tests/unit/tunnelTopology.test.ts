// node --test --import ./tests/unit/register-ts.mjs tests/unit/*.test.ts
import assert from "node:assert/strict"
import { test } from "node:test"

import * as topo from "../../cdc-native-ui/utils/tunnelTopology.ts"

const t = (id: string, status: any) => ({ id, name: id, address: "10.66.1.2/32", site_address: "10.66.1.1/32", peer_endpoint: "vpn:51820", status })

test("topology: sites ring the gateway, first one at the top", () => {
  const l = topo.layoutTopology([t("a", null), t("b", null), t("c", null), t("d", null)], 800, 500)
  assert.equal(l.cx, 400)
  assert.equal(l.cy, 250)
  const [a, b] = l.sites
  assert.ok(Math.abs(a.x - 400) < 0.01 && a.y < 250, "first site straight above the gateway")
  assert.ok(b.x > 400 && Math.abs(b.y - 250) < 0.01, "second site to the right")
  for (const s of l.sites) assert.ok(Math.abs(Math.hypot(s.x - l.cx, s.y - l.cy) - l.r1) < 0.01)
})

test("topology: each site's databases form one list on its outward side, engine from the port", () => {
  const fw = [{ host: "192.168.10.5", port: 5432, active_connections: 2 }, { host: "192.168.10.6", port: 6379 }]
  const sites = [t("top", { connected: true, handshake_age_secs: 5, forwards: fw }), t("right", null), t("bottom", null), t("left", { connected: true, handshake_age_secs: 5, forwards: [{ host: "10.0.0.1", port: 27017 }] })]
  const l = topo.layoutTopology(sites, 1000, 500)
  assert.equal(l.groups.length, 2)
  const [top, left] = l.groups
  assert.deepEqual(top.items.map((d) => d.engine), ["PostgreSQL", "Redis"])
  assert.equal(top.items[0].active, 2)
  assert.equal(top.side, "right")
  assert.equal(left.side, "left")
  assert.ok(left.x < l.sites[3].x, "left site's list is further left")
  assert.equal(l.sites[0].labelAbove, true, "top site's label goes above so the centre line does not cross it")
  assert.equal(l.sites[2].labelAbove, false)
  for (const g of l.groups) {
    assert.ok(g.y - topo.groupHeight(g.items.length) / 2 >= 0, "kept inside vertically")
    const [from, to] = g.side === "left" ? [g.x - topo.GROUP_WIDTH, g.x] : [g.x, g.x + topo.GROUP_WIDTH]
    assert.ok(from >= 0 && to <= 1000, "the whole list fits horizontally")
  }
  assert.equal(topo.engineOf(8080), "Port 8080")
})

test("topology: status tones", () => {
  assert.equal(topo.siteTone(t("x", { connected: true, handshake_age_secs: 3 })), "ok")
  assert.equal(topo.siteTone(t("x", { connected: false, handshake_age_secs: null })), "warn")
  assert.equal(topo.siteTone(t("x", { connected: false, handshake_age_secs: 900 })), "err")
  assert.equal(topo.siteTone(t("x", null)), "idle")
})
