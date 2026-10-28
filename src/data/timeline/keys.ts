/** Query keys of the Timeline. */
export const timelineKeys = {
  /** A dragged window, against the rest of the visible range. */
  window: (connectionId: string, fromMs: number, toMs: number, rangeFromMs: number) =>
    ["timelineWindow", connectionId, fromMs, toMs, rangeFromMs] as const,
}
