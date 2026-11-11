// node --test --import ./tests/unit/register-ts.mjs tests/unit/*.test.ts
import assert from "node:assert/strict"
import { test } from "node:test"

import * as g from "../../cdc-native-ui/utils/tunnelGraph.ts"

const tun = (id: string, status: any) => ({ id, name: id, address: "10.66.1.2/32", site_address: "10.66.1.1/32", peer_endpoint: "vpn:51820", status })
const tunnels = [
  tun("bkk", { connected: true, handshake_age_secs: 4, forwards: [{ host: "192.168.10.5", port: 5432, active_connections: 1 }, { host: "192.168.10.6", port: 6379 }] }),
  tun("sgp", { connected: true, handshake_age_secs: 4, forwards: [{ host: "10.66.2.1", port: 5432 }] }),
  tun("cnx", { connected: false, handshake_age_secs: null }),
]

test("graph: gateway, sites and databases with degrees; live edges only with traffic", () => {
  const gr = g.buildGraph(tunnels)
  assert.equal(gr.nodes.length, 1 + 3 + 3)
  assert.equal(gr.edges.length, 3 + 3)
  const deg = Object.fromEntries(gr.nodes.map((n) => [n.id, n.degree]))
  assert.equal(deg[g.GATEWAY_ID], 3)
  assert.equal(deg.bkk, 3)
  assert.equal(gr.edges.filter((e) => e.live).length, 3, "two connected tunnels + one database with an open connection")
  assert.ok(g.nodeRadius(gr.nodes.find((n) => n.id === "bkk")!) > g.nodeRadius(gr.nodes.find((n) => n.id === "cnx")!), "more connections, bigger node")
})

test("graph: force layout is deterministic, keeps nodes apart and inside the box", () => {
  const gr = g.buildGraph(tunnels)
  const a = g.forceLayout(gr, 900, 600)
  const b = g.forceLayout(gr, 900, 600)
  assert.deepEqual([...a.entries()], [...b.entries()], "same input, same picture")
  assert.deepEqual(a.get(g.GATEWAY_ID), { x: 450, y: 300 })
  const pts = [...a.values()]
  for (const p of pts) assert.ok(p.x >= 0 && p.x <= 900 && p.y >= 0 && p.y <= 600)
  for (let i = 0; i < pts.length; i++) for (let j = i + 1; j < pts.length; j++) assert.ok(Math.hypot(pts[i].x - pts[j].x, pts[i].y - pts[j].y) > 30, "nodes do not overlap")
  // A site's databases end up nearer their own site than the gateway does.
  const bkk = a.get("bkk")!
  const db = a.get("bkk|192.168.10.5|5432")!
  assert.ok(Math.hypot(db.x - bkk.x, db.y - bkk.y) < Math.hypot(db.x - 450, db.y - 300))
})

test("graph: GEXF for Gephi has nodes, edges, attributes and viz", () => {
  const gr = g.buildGraph(tunnels)
  const x = g.toGexf(gr, g.forceLayout(gr, 900, 600), new Date("2026-11-07T00:00:00Z"))
  assert.match(x, /^<\?xml version="1.0" encoding="UTF-8"\?>/)
  assert.match(x, /<gexf xmlns="http:\/\/gexf.net\/1.3"/)
  assert.equal((x.match(/<node /g) || []).length, 7)
  assert.equal((x.match(/<edge /g) || []).length, 6)
  assert.match(x, /<attvalue for="status" value="no handshake"\/>/)
  assert.match(x, /<viz:color r="16" g="185" b="129"\/>/)
  assert.match(x, /lastmodifieddate="2026-11-07"/)
  assert.ok(!g.toGexf({ nodes: [{ id: "a&b", kind: "site", label: "<x>", sub: "", tone: "ok", degree: 0 }], edges: [] }, new Map()).includes("<x>"), "labels are escaped")
})
