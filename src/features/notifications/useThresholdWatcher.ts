/**
 * Watches queue counters across all connections and
 * fires native OS notifications when a threshold is crossed.
 *
 * Mounted at the root so the watcher runs regardless of the active
 * route. Reads thresholds from `kv_settings` (key `notifications.thresholds`)
 * + a TanStack Query-cached `listConnections` + per-connection
 * `listQueues`. Edge-triggered via the `thresholds` helper.
 */

import { useQuery, useQueryClient } from "@tanstack/react-query"
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification"
import { useEffect, useRef } from "react"
import { fetchConnections } from "@/data/connections/queries"
import { queueKeys } from "@/data/queues/keys"
import { fetchQueues } from "@/data/queues/queries"
import { notificationThresholds } from "@/data/settings/settings"
import { useSetting } from "@/data/settings/useSetting"
import { countOf } from "@/domain/stateCounts"
import {
  isThresholdCrossing,
  type NotificationThreshold,
  thresholdKey,
  thresholdMatches,
} from "@/domain/thresholds"

export function useThresholdsState() {
  // A setting query (`["kv", …]`), so an edit in the Settings window
  // reaches the watcher of every window (`settingsSync`).
  const { stored, set, query } = useSetting(notificationThresholds)
  return { thresholds: stored ?? EMPTY, loaded: query.isFetched, persist: set }
}

const EMPTY: NotificationThreshold[] = []

/** Counters are looked at this long after the previous look ended. */
const WATCH_EVERY_MS = 5_000

/**
 * Root-mounted watcher. Polls all connections' queues every 5s
 * (lighter than the per-connection 1s actor poll; notifications
 * don't need millisecond precision).
 */
export function useThresholdWatcher() {
  const { thresholds, loaded } = useThresholdsState()
  const lastSeen = useRef<Map<string, number>>(new Map())
  const granted = useRef<boolean | null>(null)
  const queryClient = useQueryClient()

  const connectionsQuery = useQuery({
    queryKey: ["notifications.connections"],
    queryFn: fetchConnections,
    refetchInterval: 60_000,
    enabled: loaded && thresholds.length > 0,
  })

  useEffect(() => {
    if (!loaded || thresholds.length === 0 || !connectionsQuery.data) return
    let cancelled = false

    const fire = async (title: string, body: string) => {
      if (granted.current === null) {
        granted.current = (await isPermissionGranted()) || (await requestPermission()) === "granted"
      }
      if (!granted.current) return
      sendNotification({ title, body })
    }

    // Every connection at once, through the `queueKeys.list(id)`
    // cache: a list read in the last 5 s (by the sidebar or the open screen)
    // is reused, one in flight is shared. The next round starts 5 s after
    // this one ends, so a slow connection never piles rounds up. It keeps
    // going while the window is hidden: that is when a notification matters.
    const tick = async () => {
      if (cancelled) return
      const connections = connectionsQuery.data ?? []
      const lists = await Promise.allSettled(
        connections.map((conn) =>
          queryClient.fetchQuery({
            queryKey: queueKeys.list(conn.id),
            queryFn: () => fetchQueues(conn.id),
            staleTime: WATCH_EVERY_MS,
          }),
        ),
      )
      if (cancelled) return
      connections.forEach((conn, i) => {
        const r = lists[i]
        if (r?.status !== "fulfilled") return
        for (const q of r.value) {
          for (const t of thresholds) {
            if (!thresholdMatches(t, conn.id, q.name)) continue
            const current = countOf(q.counts, t.metric)
            const key = thresholdKey(conn.id, q.name, t.metric)
            const previous = lastSeen.current.get(key)
            if (isThresholdCrossing(t, previous, current)) {
              void fire(
                `Ranch: ${q.name}`,
                `${t.metric} > ${t.value} (now ${current}) on ${conn.name}`,
              )
            }
            lastSeen.current.set(key, current)
          }
        }
      })
    }

    let timer: ReturnType<typeof setTimeout> | undefined
    const loop = async () => {
      await tick()
      if (!cancelled) timer = setTimeout(() => void loop(), WATCH_EVERY_MS)
    }
    void loop()
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [loaded, thresholds, connectionsQuery.data, queryClient])
}
