// Gephi-style view of the WireGuard topology (ADR 0010): a force-directed graph (ForceAtlas2-like:
// degree-weighted repulsion, linear attraction along edges, gravity to the centre) of the gateway,
// the sites and the databases used through them, plus a GEXF export to open it in Gephi itself.
// Deterministic, so the picture does not jump between refreshes. Tested in tests/unit/tunnelGraph.test.ts.
// Point lives in dragNodes only: Nuxt auto-imports utils/ exports, so a second
// `export type Point` here triggers a "Duplicated imports" build warning.
import type { Point } from "./dragNodes"
import type { Tone } from "./transferView"
import { engineOf, siteTone, type TopoTunnel } from "./tunnelTopology"

export type GraphNode = { id: string; kind: "gateway" | "site" | "db"; label: string; sub: string; tone: Tone; degree: number; siteId?: string; active?: number }
export type GraphEdge = { source: string; target: string; live: boolean; tone: Tone }
export type Graph = { nodes: GraphNode[]; edges: GraphEdge[] }

export const GATEWAY_ID = "gateway"

export function buildGraph(tunnels: TopoTunnel[], gatewayReachable = true): Graph {
  const nodes: GraphNode[] = [{ id: GATEWAY_ID, kind: "gateway", label: "Hyper CDC gateway", sub: "cdc-wireguard", tone: gatewayReachable ? "ok" : "err", degree: 0 }]
  const edges: GraphEdge[] = []
  for (const t of tunnels) {
    const tone = siteTone(t)
    nodes.push({ id: t.id, kind: "site", label: t.name, sub: (t.site_address || "").split("/")[0] || t.peer_endpoint, tone, degree: 0 })
    edges.push({ source: GATEWAY_ID, target: t.id, live: tone === "ok", tone })
    for (const f of t.status?.forwards || []) {
      const id = `${t.id}|${f.host}|${f.port}`
      nodes.push({ id, kind: "db", label: engineOf(f.port), sub: `${f.host}:${f.port}`, tone, degree: 0, siteId: t.id, active: f.active_connections || 0 })
      edges.push({ source: t.id, target: id, live: tone === "ok" && (f.active_connections || 0) > 0, tone })
    }
  }
  const byId = new Map(nodes.map((n) => [n.id, n]))
  for (const e of edges) {
    byId.get(e.source)!.degree++
    byId.get(e.target)!.degree++
  }
  return { nodes, edges }
}

/** Node radius in px: grows with connections, like Gephi's size-by-degree. */
export const nodeRadius = (n: GraphNode) => (n.kind === "gateway" ? 26 : n.kind === "site" ? 14 + Math.min(10, n.degree * 2) : 9)

/**
 * ForceAtlas2-like layout in a width × height box. The gateway is pinned at the centre; sites start
 * on a circle, databases next to their site; then `iterations` steps of forces with cooling.
 */
export function forceLayout(
  g: Graph,
  width: number,
  height: number,
  iterations = 400,
  /** Bands kept clear (toolbar on top, legend at the bottom). */
  pad: { top: number; bottom: number } = { top: 8, bottom: 8 },
): Map<string, Point> {
  const cx = width / 2
  const cy = pad.top + (height - pad.top - pad.bottom) / 2
  const pos = new Map<string, Point>()
  const sites = g.nodes.filter((n) => n.kind === "site")
  const start = Math.min(width, height) * 0.25
  sites.forEach((s, i) => {
    const a = -Math.PI / 2 + (2 * Math.PI * i) / Math.max(1, sites.length)
    pos.set(s.id, { x: cx + start * Math.cos(a), y: cy + start * Math.sin(a) })
  })
  const perSite = new Map<string, number>()
  for (const n of g.nodes) {
    if (n.kind === "gateway") pos.set(n.id, { x: cx, y: cy })
    if (n.kind === "db") {
      const p = pos.get(n.siteId!)!
      const k = perSite.get(n.siteId!) || 0
      perSite.set(n.siteId!, k + 1)
      const a = Math.atan2(p.y - cy, p.x - cx) + (k - 1) * 0.6 // deterministic fan-out
      pos.set(n.id, { x: p.x + 60 * Math.cos(a), y: p.y + 60 * Math.sin(a) })
    }
  }
  // Fruchterman–Reingold balance with ForceAtlas2's degree weighting: repulsion k²/d (busier nodes
  // push harder), attraction d²/k along edges, light gravity so disconnected parts stay in view.
  const k = Math.sqrt((width * height) / Math.max(4, g.nodes.length)) * 0.55
  const gravity = 0.015
  for (let it = 0; it < iterations; it++) {
    const step = Math.max(0.5, k * 0.2 * (1 - it / iterations))
    const force = new Map(g.nodes.map((n) => [n.id, { x: 0, y: 0 }]))
    for (let i = 0; i < g.nodes.length; i++) {
      for (let j = i + 1; j < g.nodes.length; j++) {
        const a = g.nodes[i]
        const b = g.nodes[j]
        const pa = pos.get(a.id)!
        const pb = pos.get(b.id)!
        let dx = pa.x - pb.x
        let dy = pa.y - pb.y
        let d = Math.hypot(dx, dy)
        if (d < 0.01) {
          dx = 0.01 * (i - j)
          dy = 0.01
          d = 0.02
        }
        const f = ((k * k) / d) * Math.sqrt((a.degree + 1) * (b.degree + 1))
        const fa = force.get(a.id)!
        const fb = force.get(b.id)!
        fa.x += (dx / d) * f
        fa.y += (dy / d) * f
        fb.x -= (dx / d) * f
        fb.y -= (dy / d) * f
      }
    }
    for (const e of g.edges) {
      const pa = pos.get(e.source)!
      const pb = pos.get(e.target)!
      const dx = pb.x - pa.x
      const dy = pb.y - pa.y
      const d = Math.hypot(dx, dy) || 0.01
      const f = (d * d) / k
      const fa = force.get(e.source)!
      const fb = force.get(e.target)!
      fa.x += (dx / d) * f
      fa.y += (dy / d) * f
      fb.x -= (dx / d) * f
      fb.y -= (dy / d) * f
    }
    for (const n of g.nodes) {
      if (n.kind === "gateway") continue
      const p = pos.get(n.id)!
      const f = force.get(n.id)!
      f.x += (cx - p.x) * gravity * k
      f.y += (cy - p.y) * gravity * k
      const m = Math.hypot(f.x, f.y) || 1
      const s = Math.min(step, m)
      const mx = nodeRadius(n) + 70 // room for the label beside the box edge
      const my = nodeRadius(n) + 36
      p.x = Math.min(width - mx, Math.max(mx, p.x + (f.x / m) * s))
      p.y = Math.min(height - pad.bottom - my, Math.max(pad.top + nodeRadius(n) + 8, p.y + (f.y / m) * s))
    }
  }
  return pos
}

/** Quadratic curve between two points, bent to one side (Gephi-style curved edges). */
export function curvePath(a: Point, b: Point, bend = 0.15): string {
  const mx = (a.x + b.x) / 2
  const my = (a.y + b.y) / 2
  const qx = mx - (b.y - a.y) * bend
  const qy = my + (b.x - a.x) * bend
  return `M ${a.x.toFixed(1)} ${a.y.toFixed(1)} Q ${qx.toFixed(1)} ${qy.toFixed(1)} ${b.x.toFixed(1)} ${b.y.toFixed(1)}`
}

const GEXF_COLOR: Record<Tone, [number, number, number]> = {
  ok: [16, 185, 129],
  warn: [245, 158, 11],
  err: [239, 68, 68],
  run: [16, 185, 129],
  idle: [161, 161, 170],
}
const xml = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;")

/** GEXF 1.3 (Gephi's format) with node kind/status attributes and viz colour/size/position. */
export function toGexf(g: Graph, pos: Map<string, Point>, generated = new Date()): string {
  const attrs = `    <attributes class="node">
      <attribute id="kind" title="kind" type="string"/>
      <attribute id="status" title="status" type="string"/>
      <attribute id="address" title="address" type="string"/>
    </attributes>`
  const STATUS: Record<Tone, string> = { ok: "connected", warn: "no handshake", err: "disconnected", run: "connected", idle: "not on gateway" }
  const nodes = g.nodes
    .map((n) => {
      const p = pos.get(n.id) || { x: 0, y: 0 }
      const [r, gg, b] = n.kind === "gateway" ? [226, 151, 73] : GEXF_COLOR[n.tone]
      return `      <node id="${xml(n.id)}" label="${xml(n.label)}">
        <attvalues><attvalue for="kind" value="${n.kind}"/><attvalue for="status" value="${STATUS[n.tone]}"/><attvalue for="address" value="${xml(n.sub)}"/></attvalues>
        <viz:color r="${r}" g="${gg}" b="${b}"/><viz:size value="${nodeRadius(n)}"/><viz:position x="${p.x.toFixed(1)}" y="${(-p.y).toFixed(1)}" z="0"/>
      </node>`
    })
    .join("\n")
  const edges = g.edges.map((e, i) => `      <edge id="${i}" source="${xml(e.source)}" target="${xml(e.target)}"/>`).join("\n")
  return `<?xml version="1.0" encoding="UTF-8"?>
<gexf xmlns="http://gexf.net/1.3" xmlns:viz="http://gexf.net/1.3/viz" version="1.3">
  <meta lastmodifieddate="${generated.toISOString().slice(0, 10)}"><creator>Hyper CDC</creator><description>WireGuard topology: gateway, sites, databases in use</description></meta>
  <graph defaultedgetype="undirected" mode="static">
${attrs}
    <nodes>
${nodes}
    </nodes>
    <edges>
${edges}
    </edges>
  </graph>
</gexf>
`
}
