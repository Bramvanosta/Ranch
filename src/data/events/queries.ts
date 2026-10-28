import { useEffect, useRef, useState } from "react"

import { commands, type EventCursor, type QueueEvent } from "@/lib/tauri/bindings"

/** How often the overview reads the streams while it is on screen. */
export const LIVE_EVENTS_MS = 2_000
/** Lines kept on screen. */
const KEEP = 150

/**
 * The events of a connection's queues as they
 * come: read every 2 s from where the last read stopped, only while the
 * view is mounted and the window visible, and not while paused. Stopping
 * is simply not asking: the backend holds no subscription.
 */
export function useLiveEvents(connectionId: string, paused: boolean) {
  const [events, setEvents] = useState<QueueEvent[]>([])
  const [sampled, setSampled] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const cursors = useRef<EventCursor[]>([])
  const inFlight = useRef(false)

  // A new connection starts over.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset on connection change only.
  useEffect(() => {
    cursors.current = []
    setEvents([])
    setSampled(false)
    setError(null)
  }, [connectionId])

  useEffect(() => {
    if (paused) return
    let alive = true
    const tick = async () => {
      if (inFlight.current || document.visibilityState !== "visible") return
      inFlight.current = true
      try {
        const r = await commands.readQueueEvents({
          connectionId,
          cursors: cursors.current,
          limit: 100,
        })
        if (!alive) return
        if (r.status === "error") {
          setError(r.error.message)
          return
        }
        setError(null)
        cursors.current = r.data.cursors
        if (r.data.sampled) setSampled(true)
        if (r.data.events.length > 0) {
          setEvents((old) => {
            const seen = new Set(old.map(keyOf))
            const fresh = r.data.events.filter((e) => !seen.has(keyOf(e)))
            return [...fresh, ...old].slice(0, KEEP)
          })
        }
      } catch (e) {
        if (alive) setError(e instanceof Error ? e.message : String(e))
      } finally {
        inFlight.current = false
      }
    }
    void tick()
    const id = window.setInterval(() => void tick(), LIVE_EVENTS_MS)
    const onVisible = () => {
      if (document.visibilityState === "visible") void tick()
    }
    document.addEventListener("visibilitychange", onVisible)
    return () => {
      alive = false
      window.clearInterval(id)
      document.removeEventListener("visibilitychange", onVisible)
    }
  }, [connectionId, paused])

  return { events, sampled, error }
}

const keyOf = (e: QueueEvent) => `${e.queueName}\u0000${e.streamId}`
