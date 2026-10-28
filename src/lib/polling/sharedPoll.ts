import { hashKey, type QueryClient, type QueryKey, useQueryClient } from "@tanstack/react-query"
import { useEffect, useRef } from "react"

/**
 * One poller per query, whatever the number of components
 * showing it.
 *
 * TanStack Query runs a `refetchInterval` timer per observer: ten components
 * on the queue list meant ten timers out of phase, so one or two reads per
 * second instead of one. Here a query is polled by a single loop:
 *
 *  - **self-paced**: the next read is scheduled when the previous one ends;
 *  - **as often as the most eager subscriber asks** (a screen 1 s, the
 *    sidebar 10 s → 1 s; the sidebar alone → 10 s);
 *  - **never faster than 3 × the last read took**: on a slow link the reads
 *    space out instead of piling up;
 *  - **paused while the window is hidden**, read again at once when it is
 *    shown (the "refresh on focus" polled views need).
 *
 * Components keep their `useQuery` (cache, data, errors) with
 * `refetchInterval: false`, and call {@link useSharedPoll} with its key.
 */

interface Loop {
  client: QueryClient
  queryKey: QueryKey
  /** subscriber id → the interval it asks for */
  subscribers: Map<number, number>
  timer: ReturnType<typeof setTimeout> | null
  /** When the pending timer fires (ms epoch), to bring it forward. */
  dueAt: number
  inFlight: boolean
  /** Hidden when its turn came: read again when shown. */
  waitingForVisible: boolean
  lastDurationMs: number
}

/** The slowest a read may be paced to, whatever it took (ms). */
const MAX_DELAY_MS = 60_000

const loops = new Map<string, Loop>()
let nextId = 1

const hidden = () => typeof document !== "undefined" && document.visibilityState === "hidden"

function delayOf(loop: Loop): number {
  const wanted = Math.min(...loop.subscribers.values())
  return Math.min(MAX_DELAY_MS, Math.max(wanted, loop.lastDurationMs * 3))
}

function schedule(loop: Loop, delayMs: number) {
  if (loop.timer !== null) clearTimeout(loop.timer)
  loop.dueAt = Date.now() + delayMs
  loop.timer = setTimeout(() => void tick(loop), delayMs)
}

/**
 * A mutation declares the data it is changing with
 * `meta: { holdsPoll: (variables) => queryKey }`. While it is pending, that
 * data's poll waits a turn: a read sent before the change landed would put
 * the old value back over the optimistic one (a flicker).
 */
export type HoldsPoll = (variables: unknown) => QueryKey

const isPrefix = (prefix: QueryKey, key: QueryKey) =>
  prefix.length <= key.length && prefix.every((part, i) => hashKey([part]) === hashKey([key[i]]))

function heldByMutation(loop: Loop): boolean {
  return loop.client
    .getMutationCache()
    .getAll()
    .some((m) => {
      if (m.state.status !== "pending") return false
      const holds = m.options.meta?.holdsPoll as HoldsPoll | undefined
      return holds !== undefined && isPrefix(holds(m.state.variables), loop.queryKey)
    })
}

async function tick(loop: Loop) {
  loop.timer = null
  if (loop.subscribers.size === 0 || loop.inFlight) return
  if (hidden()) {
    loop.waitingForVisible = true
    return
  }
  if (heldByMutation(loop)) {
    schedule(loop, delayOf(loop))
    return
  }
  loop.inFlight = true
  const started = performance.now()
  try {
    await loop.client.refetchQueries(
      { queryKey: loop.queryKey, exact: true, type: "active" },
      { cancelRefetch: false },
    )
  } catch {
    // A failed read is the query's error to show; the loop goes on.
  } finally {
    loop.inFlight = false
    loop.lastDurationMs = performance.now() - started
  }
  if (loop.subscribers.size > 0) schedule(loop, delayOf(loop))
}

function onVisibilityChange() {
  if (hidden()) return
  for (const loop of loops.values()) {
    if (loop.waitingForVisible) {
      loop.waitingForVisible = false
      void tick(loop)
    }
  }
}

let listening = false
function listenToVisibility() {
  if (listening || typeof document === "undefined") return
  listening = true
  document.addEventListener("visibilitychange", onVisibilityChange)
}

/** Subscribe to the loop of `queryKey`; returns the unsubscribe. */
export function subscribePoll(
  client: QueryClient,
  queryKey: QueryKey,
  intervalMs: number,
): () => void {
  listenToVisibility()
  const id = nextId++
  const hash = hashKey(queryKey)
  let loop = loops.get(hash)
  if (!loop || loop.client !== client) {
    loop = {
      client,
      queryKey,
      subscribers: new Map(),
      timer: null,
      dueAt: 0,
      inFlight: false,
      waitingForVisible: false,
      lastDurationMs: 0,
    }
    loops.set(hash, loop)
  }
  const current = loop
  current.subscribers.set(id, intervalMs)
  // First subscriber: the mount reads it, the loop takes the next turn. A
  // more eager subscriber brings the pending turn forward.
  const delay = delayOf(current)
  if (current.timer === null && !current.inFlight) schedule(current, delay)
  else if (current.timer !== null && Date.now() + delay < current.dueAt) schedule(current, delay)

  return () => {
    current.subscribers.delete(id)
    if (current.subscribers.size === 0) {
      if (current.timer !== null) clearTimeout(current.timer)
      current.timer = null
      if (loops.get(hash) === current) loops.delete(hash)
    }
  }
}

/**
 * Poll `queryKey` every `intervalMs` (at most; see the module) while the
 * component is mounted; `false` = not polled (auto-refresh off, disabled).
 */
export function useSharedPoll(queryKey: QueryKey, intervalMs: number | false) {
  const client = useQueryClient()
  const hash = hashKey(queryKey)
  const keyRef = useRef(queryKey)
  keyRef.current = queryKey
  // biome-ignore lint/correctness/useExhaustiveDependencies: the key's hash stands for the key.
  useEffect(() => {
    if (intervalMs === false) return
    return subscribePoll(client, keyRef.current, intervalMs)
  }, [client, hash, intervalMs])
}

/** {@link useSharedPoll} for a list of queries (one per queue). */
export function useSharedPolls(queryKeys: QueryKey[], intervalMs: number | false) {
  const client = useQueryClient()
  const hashes = queryKeys.map(hashKey).join("|")
  const keysRef = useRef(queryKeys)
  keysRef.current = queryKeys
  // biome-ignore lint/correctness/useExhaustiveDependencies: the hashes stand for the keys.
  useEffect(() => {
    if (intervalMs === false) return
    const unsubscribes = keysRef.current.map((key) => subscribePoll(client, key, intervalMs))
    return () => {
      for (const unsubscribe of unsubscribes) unsubscribe()
    }
  }, [client, hashes, intervalMs])
}

/** Test seam: forget every loop. */
export function resetPollsForTests() {
  for (const loop of loops.values()) if (loop.timer !== null) clearTimeout(loop.timer)
  loops.clear()
}
