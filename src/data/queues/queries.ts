import { useQuery } from "@tanstack/react-query"
import { errorKind, unwrap } from "@/data/result"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands, type Queue, type QueueId, type QueueMaintenance } from "@/lib/tauri/bindings"
import { LIVE_SAFETY_NET_MS, useIsLive } from "@/stores/useLiveStore"
import { queueKeys } from "./keys"

/**
 * TanStack Query hook for the queue list.
 *
 * Polls `list_queues` every second by default (Settings › Refresh sets
 * another interval, or none) while the document is visible. When the user
 * switches to another tab or desktop, polling pauses and resumes on focus:
 * Redis is only asked while the user is looking.
 *
 * Schema-not-recognized failures don't retry; surface immediately so
 * the user sees the typed message. Transient failures retry once: the next
 * poll is the next try, and the error shows sooner.
 */
/** The cheapest poll there is (one pipeline per queue): may run every second. */
export const QUEUES_REFRESH_FLOOR_MS = 1_000

/** `list_queues` for `queueKeys.list(connectionId)`, shared by every reader of the key. */
export async function fetchQueues(connectionId: string) {
  return unwrap(await commands.listQueues({ connectionId }))
}

/** A connection not on screen (the sidebar): at most this often. */
export const BACKGROUND_QUEUES_FLOOR_MS = 10_000

export function useQueues(connectionId: string, opts: { background?: boolean } = {}) {
  const setting = useRefetchInterval(QUEUES_REFRESH_FLOOR_MS)
  // Followed live, the counters are read when a queue moves:
  // the poll is a safety net.
  const live = useIsLive(connectionId)
  const interval = setting === false || !live ? setting : Math.max(setting, LIVE_SAFETY_NET_MS)
  // One self-paced poller for the key, as often as its most
  // eager reader asks: the open connection's screen every second, the
  // sidebar alone every 10 s.
  useSharedPoll(
    queueKeys.list(connectionId),
    interval === false || !opts.background
      ? interval
      : Math.max(interval, BACKGROUND_QUEUES_FLOOR_MS),
  )
  return useQuery({
    queryKey: queueKeys.list(connectionId),
    queryFn: () => fetchQueues(connectionId),
    refetchInterval: false,
    retry: (failureCount, error) => {
      const kind = errorKind(error)
      // Don't retry on schemaNotRecognized; surface immediately.
      if (kind === "schemaNotRecognized") return false
      // Once: the next poll is the next try.
      return failureCount < 1
    },
  })
}

/** One `CLIENT LIST` per expanded connection: no need to ask every second. */
const WORKER_COUNTS_FLOOR_MS = 5_000

/**
 * Worker counts of a connection's queues, keyed by queue id;
 * `undefined` while unknown or when this Redis refuses `CLIENT LIST`.
 */
export function useQueueWorkerCounts(
  connectionId: string,
  queues: readonly Queue[] | undefined,
  enabled: boolean,
): ReadonlyMap<string, number> | undefined {
  const refetchInterval = useRefetchInterval(WORKER_COUNTS_FLOOR_MS)
  const ids = (queues ?? []).map((q) => q.id)
  // One self-paced poller per query (see `useSharedPoll`).
  const queryKey = queueKeys.workerCounts(connectionId, ids)
  useSharedPoll(queryKey, enabled && ids.length > 0 ? refetchInterval : false)
  const { data } = useQuery({
    queryKey,
    queryFn: async () => {
      return unwrap(await commands.countQueueWorkers({ connectionId, queueIds: ids }))
    },
    enabled: enabled && ids.length > 0,
    refetchInterval: false,
    retry: false,
  })
  if (!data?.supported) return undefined
  return new Map(data.counts.map((c) => [c.queueId, c.workers]))
}

/**
 * How many jobs a maintenance operation would touch, re-counted when the
 * options change. A count that decides a destructive action is never served
 * from cache.
 */
export function useMaintenancePreview(
  connectionId: string,
  queueId: QueueId,
  operation: QueueMaintenance | null,
  enabled: boolean,
) {
  return useQuery({
    queryKey: queueKeys.maintenancePreview(connectionId, queueId, operation),
    queryFn: async () => {
      if (!operation) throw new Error("closed")
      return unwrap(await commands.previewQueueMaintenance({ connectionId, queueId, operation }))
    },
    enabled: operation !== null && enabled,
    staleTime: 0,
    gcTime: 0,
    retry: false,
  })
}

/** `count_queue_workers` for `queueKeys.workerCounts`. */
export async function fetchQueueWorkerCounts(connectionId: string, queueIds: QueueId[]) {
  return unwrap(await commands.countQueueWorkers({ connectionId, queueIds }))
}
