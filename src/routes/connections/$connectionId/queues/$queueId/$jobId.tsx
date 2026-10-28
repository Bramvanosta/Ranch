import { createFileRoute, Link, useNavigate } from "@tanstack/react-router"
import { useEffect } from "react"
import { z } from "zod"
import { EnvDot } from "@/components/data/Env"
import { Toolbar } from "@/components/data/Toolbar"
import { useConnection } from "@/data/connections/queries"
import { type DrillDownTab, JobDrillDown } from "@/features/jobs"
import { useConnectionEvents } from "@/features/live"
import { useRouteQueue } from "@/features/queues"
import { LockButton } from "@/features/safety"
import { Screen } from "@/layouts/Screen"

/**
 * drill-into-job route.
 *
 * URL is the source of truth: `?tab=error` etc. The queue is the id the
 * listing gave it (`useRouteQueue`).
 */
/**
 * Extended tab vocabulary:
 *   - `result` shows the job's `returnValue` for completed jobs.
 *   - `stats` is the new name for what used to be `attempts`.
 * Legacy `attempts` deep links keep working: the URL value is
 * accepted by the schema and normalized to `stats` inside the
 * `JobDrillDown` component.
 */
// Trace, data, error, logs, raw, flow; the older names
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

export const Route = createFileRoute("/connections/$connectionId/queues/$queueId/$jobId")({
  component: JobDrillDownRoute,
  validateSearch: searchSchema,
})

function JobDrillDownRoute() {
  const { connectionId, queueId: param, jobId } = Route.useParams()
  const search = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  // Follow this connection's event streams while it is on screen.
  useConnectionEvents(connectionId)
  const connection = useConnection(connectionId)
  const { queueId, queueName, moveTo } = useRouteQueue(connectionId, param)
  useEffect(() => {
    if (moveTo)
      navigate({
        params: { connectionId, queueId: moveTo, jobId },
        search: (s) => s,
        replace: true,
      })
  }, [moveTo, connectionId, jobId, navigate])

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
                className="flex items-center gap-1.5 text-(--text2) hover:text-(--text) hover:underline"
                title="Back to the connection"
              >
                {/* The connection, as on the queue's page: several can share a
                    name, the environment dot tells them apart. */}
                ←{" "}
                {connection ? (
                  <>
                    <EnvDot color={connection.envColor} label={connection.envLabel} />
                    {connection.name}
                  </>
                ) : (
                  "Queues"
                )}
              </Link>
              <span className="text-(--text2)">/</span>
              <Link
                to="/connections/$connectionId/queues/$queueId"
                params={{ connectionId, queueId }}
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
        queueId={queueId}
        jobId={jobId}
        tab={search.tab}
        onTabChange={(t: DrillDownTab) => navigate({ search: { tab: t } })}
        // A removed job has no page: back to the list it was opened from.
        onRemoved={() =>
          void navigate({
            to: "/connections/$connectionId/queues/$queueId",
            params: { connectionId, queueId },
            search: { view: "jobs" },
          })
        }
      />
    </Screen>
  )
}
