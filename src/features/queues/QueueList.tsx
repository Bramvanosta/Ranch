import { Link } from "@tanstack/react-router"
import { TriangleAlertIcon } from "lucide-react"

import { Skeleton } from "@/components/ui/skeleton"
import { useQueues } from "@/data/queues/queries"
import { QueueCounters } from "./QueueCounters"

/**
 * Queue list with live counters.
 *
 * Renders four states:
 *   - **Loading** (first fetch): 3 skeleton rows.
 *   - **Empty**: the queue list returned successfully but is empty.
 *     Helpful copy hints at the BullMQ default prefix.
 *   - **Error**: render `error.message` verbatim. SchemaNotRecognized
 *     surfaces here without retry; transient failures retry up to 2x
 *     before reaching this state (TanStack Query's default).
 *   - **Populated**: list of queues, each row showing name + counter
 *     pills + reachability/pulse indicator while the next refresh is
 *     in flight.
 */
interface QueueListProps {
  connectionId: string
}

export function QueueList({ connectionId }: QueueListProps) {
  const { data, isLoading, isError, error, isFetching } = useQueues(connectionId)

  if (isLoading) {
    return (
      <ul className="flex flex-col gap-2" aria-label="Loading queues">
        {[0, 1, 2].map((i) => (
          <li key={i} className="flex items-center gap-3 rounded-md p-3">
            <Skeleton className="h-4 w-32" />
            <Skeleton className="h-5 w-64" />
          </li>
        ))}
      </ul>
    )
  }

  if (isError) {
    return (
      <div
        role="alert"
        className="flex items-start gap-2 rounded-md border border-(--danger)/40 bg-(--danger)/10 p-3 text-sm text-(--state-failed-text)"
      >
        <TriangleAlertIcon className="mt-0.5 h-4 w-4 shrink-0" />
        <div>
          <p className="font-medium">Could not list queues</p>
          <p className="mt-0.5 text-xs">
            {error instanceof Error ? error.message : "Unknown error"}
          </p>
        </div>
      </div>
    )
  }

  if (!data || data.length === 0) {
    return (
      <div className="rounded-md border border-(--line) bg-(--surf)/30 p-4 text-sm text-(--text2)">
        <p className="font-medium text-(--text)">No BullMQ queues found.</p>
        <p className="mt-1 text-xs">
          Default prefix is <code className="font-mono">bull:</code>. Custom prefixes are
          auto-detected. If your queues still don't appear, ensure jobs have been added (BullMQ
          creates the <code className="font-mono">:meta</code> hash on first job).
        </p>
      </div>
    )
  }

  return (
    <ul className="flex flex-col gap-1" aria-label="BullMQ queues">
      {data.map((q) => (
        <li key={`${q.id}`}>
          <Link
            // Each queue row drills into the per-queue job
            // list. State is omitted from the URL so the destination
            // route picks the smart default (`failed` if there are
            // failed jobs, else `waiting`).
            to="/connections/$connectionId/queues/$queueId"
            params={{ connectionId, queueId: q.id }}
            className="flex items-center justify-between gap-3 rounded-md px-3 py-2 hover:bg-(--surf)/40"
            data-testid={`queue-row-${q.name}`}
          >
            <div className="flex items-center gap-2">
              <span className="font-medium" title={`${q.id}`}>
                {q.name}
              </span>
              {q.isPaused ? (
                <span className="rounded-md bg-(--state-delayed)/15 px-2 py-0.5 text-xs text-(--state-delayed-text)">
                  paused
                </span>
              ) : null}
            </div>
            <QueueCounters counts={q.counts} isFetching={isFetching} />
          </Link>
        </li>
      ))}
    </ul>
  )
}
