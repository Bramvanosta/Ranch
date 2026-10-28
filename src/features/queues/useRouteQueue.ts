import { useQueues } from "@/data/queues/queries"
import type { Queue, QueueId } from "@/lib/tauri/bindings"

/**
 * The queue a route's `$queueId` names. The id comes from a listing and is
 * only ever compared, never taken apart. An address that names the queue
 * instead (a link kept from before queues had ids: `queues/emails`) finds
 * the queue of that name once the list is known, and `moveTo` says where
 * the route should go instead, so it does not show an empty queue.
 */
export function useRouteQueue(
  connectionId: string,
  param: string,
): { queue: Queue | undefined; queueId: QueueId; queueName: string; moveTo: QueueId | null } {
  const { data: queues } = useQueues(connectionId)
  const queue = queues?.find((q) => q.id === param)
  const named = queue ? undefined : queues?.find((q) => q.name === param)
  return {
    queue: queue ?? named,
    queueId: queue?.id ?? named?.id ?? param,
    queueName: queue?.name ?? named?.name ?? param,
    moveTo: named?.id ?? null,
  }
}
