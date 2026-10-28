import { create } from "zustand"

/**
 * The connections whose BullMQ event streams the app follows
 * (`features/live`). Polls of a followed connection slow down to a safety
 * net, since the events already say when to read again.
 */
interface LiveStore {
  live: ReadonlySet<string>
  set: (connectionId: string, on: boolean) => void
}

export const useLiveStore = create<LiveStore>((set, get) => ({
  live: new Set(),
  set: (connectionId, on) => {
    const live = new Set(get().live)
    if (on) live.add(connectionId)
    else live.delete(connectionId)
    set({ live })
  },
}))

/** Whether `connectionId`'s events are followed: polling can slow down. */
export function useIsLive(connectionId: string): boolean {
  return useLiveStore((s) => s.live.has(connectionId))
}

/** A followed connection's polls: a safety net, at most this often. */
export const LIVE_SAFETY_NET_MS = 15_000
