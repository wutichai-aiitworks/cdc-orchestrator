// WireGuard topology diagram (ADR 0010): the gateway in the centre, one node per site/tunnel on an
// inner ring, and the databases used through each tunnel (live gateway forwards) on an outer ring
// next to their site. Pure layout, unit-tested in tests/unit/tunnelTopology.test.ts.
import type { Tone } from "./transferView"

export type TopoTunnel = {
  id: string
  name: string
  address: string
  site_address?: string
  peer_endpoint: string
  status?: {
    connected: boolean
    handshake_age_secs?: number | null
    forwards?: { host: string; port: number; active_connections?: number }[]
  } | null
}

export type SiteNode = {
  id: string
  x: number
  y: number
  angle: number
  tone: Tone
  label: string
  sub: string
  /** Label above the tile (sites in the upper part) so lines from the centre never cross it. */
  labelAbove: boolean
  tunnel: TopoTunnel
}
export type DbItem = { key: string; label: string; engine: string; active: number }
/** The databases used through one tunnel: a vertical list beside its site, on the outward side. */
export type DbGroup = { siteId: string; x: number; y: number; side: "left" | "right"; items: DbItem[] }
export type Layout = { cx: number; cy: number; r1: number; sites: SiteNode[]; groups: DbGroup[] }

const ENGINE_BY_PORT: Record<number, string> = {
  5432: "PostgreSQL",
  6379: "Redis",
  27017: "MongoDB",
  3306: "MySQL",
  1433: "SQL Server",
  22: "SSH",
}
export const engineOf = (port: number) => ENGINE_BY_PORT[port] || `Port ${port}`

/** ok = connected, warn = never shook hands, err = dropped, idle = gateway does not have it. */
export function siteTone(t: TopoTunnel): Tone {
  const s = t.status
  if (!s) return "idle"
  if (s.connected) return "ok"
  return s.handshake_age_secs == null ? "warn" : "err"
}

/** Height a group of `n` database pills needs (28 px each). */
export const groupHeight = (n: number) => n * 28
/** Room a database list needs beside its anchor (widest pill: engine + host:port). */
export const GROUP_WIDTH = 230

/**
 * Radial layout in a width × height box: sites round the gateway starting at the top; each site's
 * databases as a list beside it (left of left-side sites, right otherwise), kept inside the box.
 */
export function layoutTopology(tunnels: TopoTunnel[], width: number, height: number, groupOffset = 150): Layout {
  const cx = width / 2
  const cy = height / 2
  const r1 = Math.max(110, Math.min(width * 0.28, height * 0.32))
  const n = tunnels.length
  const slice = n ? (2 * Math.PI) / n : 0
  const sites: SiteNode[] = []
  const groups: DbGroup[] = []
  tunnels.forEach((t, i) => {
    const angle = -Math.PI / 2 + i * slice
    const x = cx + r1 * Math.cos(angle)
    const y = cy + r1 * Math.sin(angle)
    sites.push({
      id: t.id,
      x,
      y,
      angle,
      tone: siteTone(t),
      label: t.name,
      sub: `${(t.site_address || "").split("/")[0] || "—"} · ${t.peer_endpoint}`,
      labelAbove: Math.sin(angle) < -0.5,
      tunnel: t,
    })
    const fw = t.status?.forwards || []
    if (fw.length) {
      const side: "left" | "right" = Math.cos(angle) < -0.2 ? "left" : "right"
      const h = groupHeight(fw.length)
      groups.push({
        siteId: t.id,
        // Lists grow away from the anchor: keep their whole width inside the box.
        x: side === "left" ? Math.max(GROUP_WIDTH + 8, x - groupOffset) : Math.min(width - GROUP_WIDTH - 8, x + groupOffset),
        y: Math.min(height - h / 2 - 8, Math.max(h / 2 + 8, y)),
        side,
        items: fw.map((f) => ({ key: `${t.id}|${f.host}|${f.port}`, label: `${f.host}:${f.port}`, engine: engineOf(f.port), active: f.active_connections || 0 })),
      })
    }
  })
  return { cx, cy, r1, sites, groups }
}
