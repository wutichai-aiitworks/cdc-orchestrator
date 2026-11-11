import { DRAG_THRESHOLD, isDrag, parseOffsets, ZERO, type Offset, type Offsets } from "~/utils/dragNodes"

/**
 * Pointer dragging for diagram nodes, with click suppression after a drag, arrow-key nudging and a
 * per-browser memory (localStorage `storageKey`). Bind `handlers(id)` on each node and wrap its
 * click handler with `guardClick`.
 */
export function useDragNodes(storageKey: string) {
  const offsets = ref<Offsets>({})
  const dragging = ref<string | null>(null)
  let press: { id: string; x: number; y: number; start: Offset; moved: boolean } | null = null
  let suppressClick = false

  onMounted(() => (offsets.value = parseOffsets(localStorage.getItem(storageKey))))
  const save = () => localStorage.setItem(storageKey, JSON.stringify(offsets.value))
  const off = (id: string): Offset => offsets.value[id] ?? ZERO
  const set = (id: string, o: Offset) => (offsets.value = { ...offsets.value, [id]: o })

  const down = (id: string, e: PointerEvent) => {
    if (e.button !== 0) return
    ;(e.currentTarget as Element).setPointerCapture?.(e.pointerId)
    press = { id, x: e.clientX, y: e.clientY, start: off(id), moved: false }
  }
  const move = (e: PointerEvent) => {
    if (!press) return
    const mx = e.clientX - press.x
    const my = e.clientY - press.y
    if (!press.moved && !isDrag(mx, my)) return
    press.moved = true
    dragging.value = press.id
    set(press.id, { dx: press.start.dx + mx, dy: press.start.dy + my })
  }
  const up = () => {
    if (press?.moved) {
      save()
      // The click that follows this pointerup belongs to the drag, not to the node.
      suppressClick = true
      setTimeout(() => (suppressClick = false), 0)
    }
    press = null
    dragging.value = null
  }
  /** Arrow keys move a focused node (Shift: further). */
  const key = (id: string, e: KeyboardEvent) => {
    const step = e.shiftKey ? 40 : 10
    const d = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[e.key]
    if (!d) return
    e.preventDefault()
    const o = off(id)
    set(id, { dx: o.dx + d[0], dy: o.dy + d[1] })
    save()
  }

  const handlers = (id: string) => ({
    onPointerdown: (e: PointerEvent) => down(id, e),
    onPointermove: move,
    onPointerup: up,
    onPointercancel: up,
    onKeydown: (e: KeyboardEvent) => key(id, e),
  })
  const guardClick = <A extends unknown[]>(fn: (...a: A) => void) => (...a: A) => {
    if (!suppressClick) fn(...a)
  }
  const moved = computed(() => Object.values(offsets.value).some((o) => o.dx || o.dy))
  const reset = () => {
    offsets.value = {}
    localStorage.removeItem(storageKey)
  }

  return { offsets, off, dragging, handlers, guardClick, moved, reset, threshold: DRAG_THRESHOLD }
}
