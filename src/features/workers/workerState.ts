import { create } from "zustand"

import type { QueueWorker } from "@/lib/tauri/bindings"

/**
 * What the Workers tab says about a worker, read from
 * its `CLIENT LIST` line: a worker waiting for a job sits in a blocking pop;
 * any other last command means it is busy with one.
 */
export type WorkerState = "working" | "idle" | "paused"

const WAITING_COMMANDS = new Set([
  "bzpopmin",
  "bzpopmax",
  "brpoplpush",
  "blmove",
  "bzmpop",
  "brpop",
  "blpop",
])

export function workerState(
  worker: Pick<QueueWorker, "lastCommand">,
  queuePaused: boolean,
): WorkerState {
  if (queuePaused) return "paused"
  const cmd = worker.lastCommand?.toLowerCase() ?? ""
  return WAITING_COMMANDS.has(cmd) ? "idle" : "working"
}

/** `ioredis 5.4.1` / `ioredis-5.4.1` / `node-redis 4.6.0` → its parts. */
export function parseLibrary(library: string | null): { name: string; version: number[] } | null {
  if (!library) return null
  const m = /^(.*?)[\s@/-]v?(\d+(?:\.\d+)*)\b/.exec(library.trim())
  if (!m) return { name: library.trim(), version: [] }
  return { name: (m[1] ?? "").trim(), version: (m[2] ?? "").split(".").map(Number) }
}

function compare(a: number[], b: number[]): number {
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const d = (a[i] ?? 0) - (b[i] ?? 0)
    if (d !== 0) return d
  }
  return 0
}

/**
 * The libraries of a connection's workers, each with whether an older
 * version than the newest of the same library is in use (worth an upgrade:
 * a mixed fleet runs two BullMQ behaviours).
 */
export function libraryVersions(libraries: ReadonlyArray<string | null>): {
  distinct: string[]
  older: Set<string>
} {
  const parsed = [...new Set(libraries.filter((l): l is string => Boolean(l)))]
  const newest = new Map<string, number[]>()
  for (const lib of parsed) {
    const p = parseLibrary(lib)
    if (!p || p.version.length === 0) continue
    const best = newest.get(p.name)
    if (!best || compare(p.version, best) > 0) newest.set(p.name, p.version)
  }
  const older = new Set<string>()
  for (const lib of parsed) {
    const p = parseLibrary(lib)
    const best = p ? newest.get(p.name) : undefined
    if (p && best && p.version.length > 0 && compare(p.version, best) < 0) older.add(lib)
  }
  return { distinct: parsed.sort(), older }
}

/**
 * When a queue last had a worker, as Ranch saw it (Redis keeps no trace of
 * a client that left). Per window; a queue never seen with one has none.
 */
interface LastSeen {
  seen: Map<string, number>
  mark: (key: string, now: number) => void
}

export const useLastWorkerSeen = create<LastSeen>((set, get) => ({
  seen: new Map(),
  mark: (key, now) => {
    const seen = new Map(get().seen)
    seen.set(key, now)
    set({ seen })
  },
}))
