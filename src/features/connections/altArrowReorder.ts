/**
 * Pure helper for the Alt+arrow keyboard reorder pattern.
 *
 * The dnd-kit KeyboardSensor's built-in pattern is Space → arrow → Space (drag
 * mode); the AC explicitly requires Alt+ArrowUp/Down without entering drag
 * mode. So we attach a regular `onKeyDown` handler to group/env headers and
 * dispatch the same reorder mutation as drag-end.
 *
 * `moveItemInList` returns a new array with `arr[fromIdx]` shifted by `delta`
 * positions, clamped at the boundaries (no wrap-around — the user gets clear
 * feedback when they hit the edge by the focus staying put).
 */
export function moveItemInList<T>(arr: readonly T[], fromIdx: number, delta: number): T[] {
  const toIdx = Math.max(0, Math.min(arr.length - 1, fromIdx + delta))
  if (toIdx === fromIdx) return arr.slice()
  const next = arr.slice()
  const [item] = next.splice(fromIdx, 1)
  next.splice(toIdx, 0, item)
  return next
}

/**
 * Returns the delta (-1, 0, +1) implied by an Alt+arrow keydown event, or
 * `null` if the event isn't an Alt+arrow we handle. Returning the delta lets
 * the caller decide whether to call `moveItemInList` + dispatch the mutation,
 * keeping side effects out of this helper.
 */
export function altArrowDelta(e: KeyboardEvent | React.KeyboardEvent): -1 | 1 | null {
  if (!e.altKey) return null
  if (e.key === "ArrowUp") return -1
  if (e.key === "ArrowDown") return 1
  return null
}
