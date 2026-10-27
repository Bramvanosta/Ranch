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

import { fetchQueues } from "@/features/queues/useQueues"
import { announceSettingsChange } from "@/lib/settingsSync"
import { commands } from "@/lib/tauri/bindings"

import {
  isThresholdCrossing,
  type NotificationThreshold,
  type ThresholdMetric,
  thresholdKey,
  thresholdMatches,
} from "./thresholds"

const THRESHOLDS_KEY = "notifications.thresholds"
const THRESHOLDS_QUERY_KEY = ["kv", THRESHOLDS_KEY] as const

export function useThresholdsState() {
  const queryClient = useQueryClient()
  // A query (`["kv", …]`), so an edit in the Settings window
  // reaches the watcher of every window (`settingsSync`).
  const query = useQuery({
    queryKey: THRESHOLDS_QUERY_KEY,
    queryFn: async (): Promise<NotificationThreshold[]> => {
      const r = await commands.getKvSetting(THRESHOLDS_KEY)
      if (r.status !== "ok" || !r.data) return []
      try {
        const parsed = JSON.parse(r.data)
        return Array.isArray(parsed) ? parsed : []
      } catch {
        return [] // corrupt — defensively ignore
      }
    },
    staleTime: Number.POSITIVE_INFINITY,
  })

  const persist = async (next: NotificationThreshold[]) => {
    queryClient.setQueryData(THRESHOLDS_QUERY_KEY, next)
    const r = await commands.setKvSetting(THRESHOLDS_KEY, JSON.stringify(next))
    if (r.status === "ok") announceSettingsChange({ topic: "kv" })
  }

  return { thresholds: query.data ?? EMPTY, loaded: query.isFetched, persist }
}

const EMPTY: NotificationThreshold[] = []

/** Counters are looked at this long after the previous look ended. */
const WATCH_EVERY_MS = 5_000

/**
 * Root-mounted watcher. Polls all connections' queues every 5s
 * (lighter than the per-connection 1s actor poll — notifications
 * don't need millisecond precision).
 */
export function useThresholdWatcher() {
  const { thresholds, loaded } = useThresholdsState()
  const lastSeen = useRef<Map<string, number>>(new Map())
  const granted = useRef<boolean | null>(null)
  const queryClient = useQueryClient()

  const connectionsQuery = useQuery({
    queryKey: ["notifications.connections"],
    queryFn: async () => {
      const r = await commands.listConnections()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
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

    // Every connection at once, through the `["queues", id]`
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
            queryKey: ["queues", conn.id],
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
            if (!thresholdMatches(t, conn.id, q.queueRef.name)) continue
            const current = (q.counters as Record<ThresholdMetric, number>)[t.metric] ?? 0
            const key = thresholdKey(conn.id, q.queueRef.name, t.metric)
            const previous = lastSeen.current.get(key)
            if (isThresholdCrossing(t, previous, current)) {
              void fire(
                `Ranch — ${q.queueRef.name}`,
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
