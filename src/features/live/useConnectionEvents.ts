import { type QueryClient, useQueryClient } from "@tanstack/react-query"
import { listen } from "@tauri-apps/api/event"
import { useEffect } from "react"
import { create } from "zustand"

import { getRuntime } from "@/lib/runtime/runtime"
import { commands, type QueueEventsNotice } from "@/lib/tauri/bindings"

/**
 * Live updates from BullMQ's event streams.
 *
 * While a connection is on screen and the window is shown, the app follows
 * it (`follow_connection_events`): Ranch reads the queues' event streams
 * (`XREAD BLOCK`, a connection of its own) and sends `queue:events` with the
 * queues that moved. Those queues' counters, job lists and open jobs are
 * read again right away; polling stays as a slow safety net (see
 * {@link useIsLive}). Hidden window, other screen: not followed.
 *
 * Desktop only: the server runtime answers `false` and the web app keeps
 * polling as before.
 */

interface LiveStore {
  live: ReadonlySet<string>
  set: (connectionId: string, on: boolean) => void
}

const useLiveStore = create<LiveStore>((set, get) => ({
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

export const QUEUE_EVENTS = "queue:events"

/** The queries a notice makes stale: only the queues that moved. */
export function refreshFor(qc: QueryClient, notice: QueueEventsNotice) {
  const id = notice.connectionId
  const keep = { cancelRefetch: false }
  void qc.invalidateQueries({ queryKey: ["queues", id] }, keep)
  void qc.invalidateQueries({ queryKey: ["recent-jobs", id] }, keep)
  const failed = notice.queues.some((q) => q.events.includes("failed"))
  if (failed) {
    void qc.invalidateQueries({ queryKey: ["failed-jobs", id] }, keep)
    void qc.invalidateQueries({ queryKey: ["failureCauses", id] }, keep)
  }
  for (const { queueRef } of notice.queues) {
    void qc.invalidateQueries({ queryKey: ["jobs", id, queueRef.prefix, queueRef.name] }, keep)
    void qc.invalidateQueries({ queryKey: ["job", id, queueRef.prefix, queueRef.name] }, keep)
  }
}

export function useConnectionEvents(connectionId: string) {
  const qc = useQueryClient()
  const setLive = useLiveStore((s) => s.set)

  useEffect(() => {
    if (getRuntime() !== "desktop") return
    let cancelled = false
    let following = false
    let unlisten: (() => void) | undefined

    const follow = async (on: boolean) => {
      if (on === following) return
      following = on
      const r = await commands.followConnectionEvents({ connectionId, follow: on })
      if (!cancelled) setLive(connectionId, on && r.status === "ok" && r.data)
    }
    const onVisibility = () => void follow(document.visibilityState === "visible")

    void listen<QueueEventsNotice>(QUEUE_EVENTS, (event) => {
      if (event.payload.connectionId === connectionId) refreshFor(qc, event.payload)
    })
      .then((fn) => {
        if (cancelled) fn()
        else unlisten = fn
      })
      .catch(() => {
        // Outside Tauri: polling carries on.
      })
    onVisibility()
    document.addEventListener("visibilitychange", onVisibility)
    return () => {
      cancelled = true
      document.removeEventListener("visibilitychange", onVisibility)
      unlisten?.()
      setLive(connectionId, false)
      if (following) void commands.followConnectionEvents({ connectionId, follow: false })
    }
  }, [connectionId, qc, setLive])
}
