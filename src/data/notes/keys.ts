/** Query keys of agents' notes on failure causes. */
export const noteKeys = {
  /** Every list of notes, whatever its connection. */
  all: ["causeNotes"] as const,
  /** One connection's notes (`null`: every connection's). */
  list: (connectionId: string | null, includeDismissed: boolean) =>
    [...noteKeys.all, connectionId ?? "all", includeDismissed] as const,
}
