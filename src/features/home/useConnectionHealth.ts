/**
 * The home page looks at every connection at once: never faster than this.
 *
 * The health of a connection is read from its queue list,
 * `["queues", id]` (see `useFleet`), shared with the sidebar's poll.
 */
export const HOME_HEALTH_FLOOR_MS = 10_000
