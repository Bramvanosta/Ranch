import { create } from "zustand"

/**
 * When each running job last changed its progress, as
 * Ranch saw it. BullMQ keeps no time for a progress update, so the list
 * remembers when the value it polls changes; a job first seen counts from
 * then. Kept per window, forgotten on restart.
 */
interface ProgressWatch {
  seen: Map<string, { value: string | null; since: number }>
  observe: (key: string, value: string | null, now: number) => void
}

export const useProgressWatchStore = create<ProgressWatch>((set, get) => ({
  seen: new Map(),
  observe: (key, value, now) => {
    const prev = get().seen.get(key)
    if (prev && prev.value === value) return
    const seen = new Map(get().seen)
    seen.set(key, { value, since: now })
    set({ seen })
  },
}))

/** No progress update for this long, with the lock still renewed: worth a look. */
export const STALL_MS = 3 * 60_000

export function progressKey(connectionId: string, queue: string, id: string): string {
  return `${connectionId}\u0000${queue}\u0000${id}`
}

/**
 * How long a running job has shown the same progress, and whether that
 * looks stuck: it reported progress once, has not moved for
 * [`STALL_MS`], and a worker still renews its lock.
 */
export function stallOf(
  watch: { value: string | null; since: number } | undefined,
  lockTtlMs: number | null | undefined,
  now: number,
): { quietMs: number | null; stalled: boolean } {
  if (!watch || watch.value === null) return { quietMs: null, stalled: false }
  const quietMs = Math.max(now - watch.since, 0)
  return { quietMs, stalled: quietMs >= STALL_MS && (lockTtlMs ?? 0) > 0 }
}
