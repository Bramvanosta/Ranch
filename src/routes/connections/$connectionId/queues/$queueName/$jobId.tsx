import { createFileRoute, Link, useNavigate } from "@tanstack/react-router"
import { useEffect } from "react"
import { z } from "zod"
import { Toolbar } from "@/components/data/Toolbar"
import { type DrillDownTab, JobDrillDown } from "@/features/jobs/JobDrillDown"
import { useConnectionEvents } from "@/features/live/useConnectionEvents"
import { useQueues } from "@/features/queues/useQueues"
import { LockButton } from "@/features/safety/LockButton"
import { Screen } from "@/layouts/Screen"
import { useSelectionStore } from "@/stores/useSelectionStore"

/**
 * drill-into-job route.
 *
 * URL is the source of truth: `?tab=error` etc. The route
 * hardcodes `prefix: "bull"` for the queueRef constructed from the URL;
 * if the queue uses a custom prefix, we look it up from `useQueues`
 * (the cached counter poll) so custom-prefix deployments still
 * resolve. Pure-URL custom-prefix deep linking lands in a v1.x story.
 */
/**
 * Extended tab vocabulary:
 *   - `result` shows the job's `returnValue` for completed jobs.
 *   - `stats` is the new name for what used to be `attempts`.
 * Legacy `attempts` deep links keep working — the URL value is
 * accepted by the schema and normalized to `stats` inside the
 * `JobDrillDown` component.
 */
// Trace · data · error · logs · raw · flow; the older names
// still open (JobDrillDown maps them).
const TABS = [
  "trace",
  "data",
  "error",
  "logs",
  "raw",
  "flow",
  "payload",
  "options",
  "stats",
  "attempts",
  "stack",
  "result",
] as const
const searchSchema = z.object({
  tab: z.enum(TABS).optional().catch(undefined),
})

export const Route = createFileRoute("/connections/$connectionId/queues/$queueName/$jobId")({
  component: JobDrillDownRoute,
  validateSearch: searchSchema,
})

function JobDrillDownRoute() {
  const { connectionId, queueName, jobId } = Route.useParams()
  const search = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  const select = useSelectionStore((s) => s.select)

  useEffect(() => {
    select(connectionId)
    return () => {
      select(null)
    }
  }, [connectionId, select])

  // Follow this connection's event streams while it is on screen.
  useConnectionEvents(connectionId)
  const { data: queues } = useQueues(connectionId)
  const queue = queues?.find((q) => q.queueRef.name === queueName)
  const queueRef = queue?.queueRef ?? { prefix: "bull", name: queueName }

  return (
    <Screen
      toolbar={
        <Toolbar
          titleAs="div"
          leading={
            <nav aria-label="Breadcrumb" className="flex items-baseline gap-2 text-sm">
              <Link
                to="/connections/$connectionId"
                params={{ connectionId }}
                className="text-(--text2) hover:text-(--text) hover:underline"
              >
                ← Queues
              </Link>
              <span className="text-(--text2)">/</span>
              <Link
                to="/connections/$connectionId/queues/$queueName"
                params={{ connectionId, queueName }}
                className="text-(--text2) hover:text-(--text) hover:underline"
              >
                {queueName}
              </Link>
              <span className="text-(--text2)">/</span>
            </nav>
          }
          title={<span className="font-mono text-sm font-normal text-(--text2)">#{jobId}</span>}
          actions={<LockButton connectionId={connectionId} />}
        />
      }
      // The page scrolls as a whole: the facts on top, then content that can
      // be long (a payload, a stack trace, logs).
      contentClassName="gap-4 overflow-y-auto"
    >
      <JobDrillDown
        connectionId={connectionId}
        queueRef={queueRef}
        jobId={jobId}
        tab={search.tab}
        onTabChange={(t: DrillDownTab) => navigate({ search: { tab: t } })}
        // A removed job has no page: back to the list it was opened from.
        onRemoved={() =>
          void navigate({
            to: "/connections/$connectionId/queues/$queueName",
            params: { connectionId, queueName },
            search: { view: "jobs" },
          })
        }
      />
    </Screen>
  )
}
