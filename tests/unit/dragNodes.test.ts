// Draggable diagram nodes (cdc-native-ui/utils/dragNodes.ts).
import test from "node:test"
import assert from "node:assert/strict"

import { isDrag, parseOffsets, placed } from "../../cdc-native-ui/utils/dragNodes.ts"

test("a press becomes a drag only after a few pixels (so clicks still select)", () => {
  assert.equal(isDrag(2, 2), false)
  assert.equal(isDrag(3, 3), true)
  assert.equal(isDrag(0, -4), true)
})

test("placed: automatic position plus offset, kept inside the diagram", () => {
  assert.deepEqual(placed({ x: 100, y: 100 }, { dx: 30, dy: -20 }, 800, 600), { x: 130, y: 80 })
  assert.deepEqual(placed({ x: 100, y: 100 }, undefined, 800, 600), { x: 100, y: 100 })
  assert.deepEqual(placed({ x: 790, y: 5 }, { dx: 500, dy: -500 }, 800, 600), { x: 784, y: 16 })
})

test("parseOffsets keeps valid entries and drops anything malformed", () => {
  assert.deepEqual(parseOffsets('{"a":{"dx":1,"dy":-2},"b":{"dx":"x","dy":1},"c":null}'), { a: { dx: 1, dy: -2 } })
  assert.deepEqual(parseOffsets("not json"), {})
  assert.deepEqual(parseOffsets("[1,2]"), {})
  assert.deepEqual(parseOffsets(null), {})
})
