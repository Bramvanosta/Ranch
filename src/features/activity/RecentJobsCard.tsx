import { Link } from "@tanstack/react-router"
import { useEffect, useState } from "react"
import { useJobStates } from "@/capabilities"
import { Panel, PanelBody, PanelHeader, PanelTitle } from "@/components/data/Panel"
import { StatePill } from "@/components/data/StatePill"
import { useRecentJobs } from "@/data/jobs/queries"
import { formatDurationMs } from "@/domain/format"
import { formatAge } from "@/domain/formatAge"
import { StaleNotice } from "@/features/freshness"
import { summarizeError } from "@/features/jobs"
import type { Job, QueueId } from "@/lib/tauri/bindings"
import { formatMoment } from "@/lib/time"

interface RecentJobsCardProps {
  connectionId: string
  /** One queue; `null` = every queue of the connection, merged. */
  queueId: QueueId | null
}

/** "ran 1.2 s" for a finished job, "for 14 s" for a running one. */
export function describeRun(job: Job, now: number): string | null {
  if (job.processedOn == null) return null
  if (job.state === "active") return `for ${formatDurationMs(Math.max(0, now - job.processedOn))}`
  if (job.finishedOn == null) return null
  return `ran ${formatDurationMs(Math.max(0, job.finishedOn - job.processedOn))}`
}

/**
 * What ran last: the running jobs, then the latest to finish, completed and
 * failed together, across queues on a connection's overview. Each row opens
 * the job.
 */
export function RecentJobsCard({ connectionId, queueId }: RecentJobsCardProps) {
  const { data, isLoading, error, dataUpdatedAt } = useRecentJobs(connectionId, queueId)
  const states = useJobStates(connectionId)
  // "12 s ago" has to move between two refetches too.
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1_000)
    return () => window.clearInterval(id)
  }, [])

  const jobs = data?.jobs ?? []
  const acrossQueues = queueId === null

  return (
    <Panel data-testid="recent-jobs">
      <PanelHeader className="pb-1">
        <PanelTitle className="text-sm font-medium">Recent jobs</PanelTitle>
        <p className="text-xs text-(--text2)">
          Running now, then the latest to finish
          {acrossQueues ? " across all queues" : ""}
        </p>
      </PanelHeader>
      <PanelBody>
        {error && data ? (
          <StaleNotice error={error} updatedAt={dataUpdatedAt} what="the recent jobs" />
        ) : null}
        {/* The error line only when there is nothing to show. */}
        {error && !data ? (
          <p
            role="alert"
            className="py-4 text-sm text-(--state-failed-text)"
            data-testid="recent-jobs-error"
          >
            Could not read the recent jobs: {error.message}
          </p>
        ) : isLoading ? (
          <p className="py-6 text-center text-sm text-(--text2)" role="status">
            Loading
          </p>
        ) : jobs.length === 0 ? (
          <p className="py-6 text-center text-sm text-(--text2)">
            Nothing is running and no finished job is kept
            {acrossQueues ? " on this connection" : " in this queue"}.
          </p>
        ) : (
          <ul className="flex flex-col divide-y divide-(--line)">
            {jobs.map((job) => {
              const at = job.finishedOn ?? job.processedOn ?? job.timestamp
              const run = describeRun(job, now)
              return (
                <li key={`${job.jobRef.queueId}\u0000${job.jobRef.id}`}>
                  <Link
                    to="/connections/$connectionId/queues/$queueId/$jobId"
                    params={{
                      connectionId,
                      queueId: job.jobRef.queueId,
                      jobId: job.jobRef.id,
                    }}
                    className="-mx-2 flex items-center gap-3 rounded-md px-2 py-1.5 text-sm hover:bg-(--surf)/60"
                    data-testid="recent-job-row"
                  >
                    {/* The pill every list uses; a fixed slot keeps the names aligned. */}
                    <span className="flex w-[5.5rem] shrink-0">
                      <StatePill state={job.state} states={states} />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="flex flex-wrap items-baseline gap-x-2">
                        <span className="min-w-0 font-medium break-words">
                          {job.name || "(unnamed)"}
                        </span>
                        <span className="min-w-0 font-mono text-xs wrap-anywhere text-(--text2)">
                          {acrossQueues ? `${job.jobRef.queueName} ` : ""}#{job.jobRef.id}
                        </span>
                      </span>
                      {job.state === "failed" && job.failedReason ? (
                        <span className="block text-xs break-words text-(--state-failed-text)">
                          {summarizeError(job.failedReason)}
                        </span>
                      ) : null}
                    </span>
                    {run ? (
                      <span className="hidden shrink-0 font-tabular text-xs text-(--text2) sm:inline">
                        {run}
                      </span>
                    ) : null}
                    <span
                      className="w-14 shrink-0 text-right font-tabular text-xs text-(--text2)"
                      title={formatMoment(at, { seconds: true })}
                    >
                      {job.state === "active" ? "now" : `${formatAge(now - at)} ago`}
                    </span>
                  </Link>
                </li>
              )
            })}
          </ul>
        )}
        {data && data.unreadableQueues.length > 0 ? (
          <p className="pt-2 text-xs text-(--text2)" data-testid="recent-jobs-unreadable">
            Could not read: {data.unreadableQueues.join(", ")}.
          </p>
        ) : null}
      </PanelBody>
    </Panel>
  )
}
