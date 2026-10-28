import { type QueryClient, useQueryClient } from "@tanstack/react-query"
import { listen } from "@tauri-apps/api/event"
import { useEffect } from "react"
import { followConnectionEvents } from "@/data/events/mutations"
import { jobKeys } from "@/data/jobs/keys"
import { queueKeys } from "@/data/queues/keys"
import { hasLiveEvents } from "@/lib/runtime/runtime"
import type { QueueEventsNotice } from "@/lib/tauri/bindings"
import { useLiveStore } from "@/stores/useLiveStore"

/**
 * Live updates from BullMQ's event streams.
 *
 * While a connection is on screen and the window is shown, the app follows
 * it (`follow_connection_events`): Ranch reads the queues' event streams
 * (`XREAD BLOCK`, a connection of its own) and sends `queue:events` with the
 * queues that moved. Those queues' counters, job lists and open jobs are
 * read again right away; polling stays as a slow safety net (see
 * `useIsLive` in `stores/useLiveStore`). Hidden window, other screen: not followed.
 *
 * Desktop only: the server runtime answers `false` and the web app keeps
 * polling as before.
 */

export const QUEUE_EVENTS = "queue:events"

/** The queries a notice makes stale: only the queues that moved. */
export function refreshFor(qc: QueryClient, notice: QueueEventsNotice) {
  const id = notice.connectionId
  const keep = { cancelRefetch: false }
  void qc.invalidateQueries({ queryKey: queueKeys.list(id) }, keep)
  void qc.invalidateQueries({ queryKey: jobKeys.recent(id) }, keep)
  const failed = notice.queues.some((q) => q.events.includes("failed"))
  if (failed) {
    void qc.invalidateQueries({ queryKey: jobKeys.failed(id) }, keep)
    void qc.invalidateQueries({ queryKey: jobKeys.failureCauses(id) }, keep)
  }
  for (const { queueId } of notice.queues) {
    void qc.invalidateQueries({ queryKey: jobKeys.lists(id, queueId) }, keep)
    void qc.invalidateQueries({ queryKey: jobKeys.details(id, queueId) }, keep)
  }
}

export function useConnectionEvents(connectionId: string) {
  const qc = useQueryClient()
  const setLive = useLiveStore((s) => s.set)

  useEffect(() => {
    if (!hasLiveEvents()) return
    let cancelled = false
    let following = false
    let unlisten: (() => void) | undefined

    const follow = async (on: boolean) => {
      if (on === following) return
      following = on
      const r = await followConnectionEvents({ connectionId, follow: on })
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
      if (following) void followConnectionEvents({ connectionId, follow: false })
    }
  }, [connectionId, qc, setLive])
}
