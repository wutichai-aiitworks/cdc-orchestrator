// Draggable diagram nodes (Tunnels → Mesh / Graph). A drag stores an offset from the node's
// automatic position, not an absolute point, so the diagram still re-flows on resize and when sites
// are added, while keeping what the user moved. Unit-tested in tests/unit/dragNodes.test.ts.

export type Offset = { dx: number; dy: number }
export type Offsets = Record<string, Offset>
export type Point = { x: number; y: number }

export const ZERO: Offset = { dx: 0, dy: 0 }
/** Pointer travel (px) before a press becomes a drag; below it, the press is a click. */
export const DRAG_THRESHOLD = 4

export const isDrag = (dx: number, dy: number) => Math.hypot(dx, dy) >= DRAG_THRESHOLD

/** Automatic position plus the user's offset, kept `pad` px inside the diagram. */
export function placed(p: Point, off: Offset | undefined, width: number, height: number, pad = 16): Point {
  const o = off ?? ZERO
  const clamp = (v: number, max: number) => Math.min(Math.max(v, pad), Math.max(pad, max - pad))
  return { x: clamp(p.x + o.dx, width), y: clamp(p.y + o.dy, height) }
}

/** Stored offsets; anything malformed is dropped rather than breaking the diagram. */
export function parseOffsets(raw: string | null): Offsets {
  try {
    const v = JSON.parse(raw || "{}")
    if (!v || typeof v !== "object" || Array.isArray(v)) return {}
    const out: Offsets = {}
    for (const [k, o] of Object.entries(v as Record<string, any>)) {
      if (o && Number.isFinite(o.dx) && Number.isFinite(o.dy)) out[k] = { dx: o.dx, dy: o.dy }
    }
    return out
  } catch {
    return {}
  }
}
