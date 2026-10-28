import { useQuery, useQueryClient } from "@tanstack/react-query"
import { Link } from "@tanstack/react-router"
import { XIcon } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"
import { useCapabilities } from "@/capabilities"
import { CodeBlock } from "@/components/data/CodeBlock"
import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { SectionHeader } from "@/components/data/SectionHeader"
import { Button } from "@/components/ui/button"
import { auditKeys } from "@/data/audit/keys"
import { jobKeys } from "@/data/jobs/keys"
import { addJob } from "@/data/jobs/mutations"
import { useLatestJobsOf } from "@/data/jobs/queries"
import { unwrap } from "@/data/result"
import { withOneShotUnlock } from "@/data/safety/oneShotUnlock"
import { useConnectionLockState } from "@/data/safety/queries"
import { schedulerKeys } from "@/data/schedulers/keys"
import { removeScheduler, usePauseScheduler, useResumeScheduler } from "@/data/schedulers/mutations"
import { fetchRunsWithin } from "@/data/schedulers/queries"
import { formatAge } from "@/domain/formatAge"
import { SchedulerSheet } from "@/features/queues"
import type { CronJob, Job, QueueId } from "@/lib/tauri/bindings"
import { formatMoment } from "@/lib/time"
import { describeSchedule, runsOf } from "./describeSchedule"
import { rowId, useConnectionSchedulers } from "./useConnectionSchedulers"

const YEAR_MS = 366 * 24 * 60 * 60 * 1000

/**
 * One scheduler beside the list: its next five runs,
 * the job it makes, its last runs (links to the jobs), and what can be
 * done: Edit (a sheet), Pause / Resume, Run once now, Remove.
 * On a protected connection a write asks for the queue's name and runs
 * under a one-shot unlock.
 */
export function SchedulerInspector({
  connectionId,
  queueId,
  queueName,
  scheduler,
  recentJobs,
  onClose,
}: {
  connectionId: string
  queueId: QueueId
  queueName: string
  scheduler: CronJob
  recentJobs: readonly Job[]
  onClose: () => void
}) {
  const qc = useQueryClient()
  const { data: lockState } = useConnectionLockState(connectionId)
  const locked = lockState === "locked"
  const pause = usePauseScheduler()
  const resume = useResumeScheduler()
  // What the backend declares: editing and removing, pausing, adding a job.
  const { can } = useCapabilities(connectionId)
  const [editing, setEditing] = useState(false)
  const [confirming, setConfirming] = useState<"run" | "remove" | null>(null)
  const [busy, setBusy] = useState(false)
  const [now] = useState(() => Date.now())

  const next = useQuery({
    queryKey: schedulerKeys.nextRuns(connectionId, queueId, scheduler),
    queryFn: () => fetchRunsWithin(scheduler, YEAR_MS, 5),
    enabled: !scheduler.paused,
  })
  const runs = runsOf(scheduler, recentJobs).slice(0, 5)
  const { main, plain } = describeSchedule(scheduler)
  const target = { connectionId, queueId, schedulerKey: scheduler.key }
  const guarded = <T,>(send: () => Promise<T>) =>
    locked ? withOneShotUnlock(connectionId, send) : send()

  const refresh = () => {
    qc.invalidateQueries({ queryKey: schedulerKeys.all(connectionId) })
    qc.invalidateQueries({ queryKey: auditKeys.schedulerPauses(connectionId) })
  }

  const confirm = async () => {
    if (!confirming) return
    setBusy(true)
    try {
      if (confirming === "run") {
        const r = unwrap(
          await guarded(() =>
            addJob({
              connectionId,
              queueId,
              job: { name: scheduler.name, data: scheduler.template ?? "{}" },
            }),
          ),
        )
        toast.success(`Job #${r.jobId} added to ${queueName}`, {
          description: `${scheduler.name} · ${r.state}`,
        })
        qc.invalidateQueries({ queryKey: jobKeys.all(connectionId) })
      } else {
        unwrap(await guarded(() => removeScheduler(target)))
        toast.success(`Scheduler ${scheduler.key} removed`)
        onClose()
      }
      setConfirming(null)
      refresh()
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="flex min-h-0 flex-col" data-testid="scheduler-inspector">
      <div className="flex items-center justify-end border-b border-(--line) px-2 py-1">
        <Button variant="ghost" size="icon-sm" aria-label="Close the inspector" onClick={onClose}>
          <XIcon aria-hidden="true" />
        </Button>
      </div>
      <div className="flex flex-col gap-5 p-4">
        <header className="flex flex-col gap-1">
          <h2 className="text-lg font-semibold break-words">{scheduler.key}</h2>
          <p className="text-sm text-(--text2)">
            makes <span className="text-(--text)">{scheduler.name}</span> on {queueName}
          </p>
          <p className="font-tabular text-xs wrap-anywhere">{main}</p>
          {plain ? <p className="text-xs text-(--text2)">{plain}</p> : null}
          <div className="mt-2 flex flex-wrap gap-1.5">
            {can("schedules.write") ? (
              <Button
                size="sm"
                variant="default"
                onClick={() => setEditing(true)}
                data-testid="scheduler-edit"
              >
                Edit
              </Button>
            ) : null}
            {can("schedules.pause") ? (
              <Button
                size="sm"
                variant="default"
                disabled={pause.isPending || resume.isPending}
                onClick={() =>
                  void guarded(() => (scheduler.paused ? resume : pause).mutateAsync(target)).then(
                    refresh,
                    (e: unknown) => toast.error(e instanceof Error ? e.message : String(e)),
                  )
                }
                data-testid="scheduler-pause-toggle"
              >
                {scheduler.paused ? "Resume" : "Pause"}
              </Button>
            ) : null}
            {can("jobActions.add") ? (
              <Button
                size="sm"
                variant="default"
                onClick={() => setConfirming("run")}
                data-testid="scheduler-run-now"
              >
                Run once now
              </Button>
            ) : null}
            {can("schedules.write") ? (
              <Button
                size="sm"
                variant="danger"
                onClick={() => setConfirming("remove")}
                data-testid="scheduler-remove"
              >
                Remove
              </Button>
            ) : null}
          </div>
        </header>

        <section aria-label="Next runs" className="flex flex-col gap-2">
          <SectionHeader title="Next runs" level={3} />
          {scheduler.paused ? (
            <p className="text-sm text-(--text2)">Paused: it runs again once resumed.</p>
          ) : (
            <ol
              className="flex flex-col gap-0.5 font-tabular text-sm"
              data-testid="scheduler-next-runs"
            >
              {(next.data ?? []).map((t) => (
                <li key={t}>{formatMoment(t)}</li>
              ))}
            </ol>
          )}
        </section>

        <section aria-label="Job template" className="flex flex-col gap-2">
          <SectionHeader title="Job template" level={3} />
          {scheduler.template ? (
            <CodeBlock label="Job template">
              <code>{pretty(scheduler.template)}</code>
            </CodeBlock>
          ) : (
            <p className="text-sm text-(--text2)">No data: the job is added empty.</p>
          )}
        </section>

        <section aria-label="Last runs" className="flex flex-col gap-2">
          <SectionHeader title="Last runs" level={3} />
          {runs.length === 0 ? (
            <p className="text-sm text-(--text2)">None among the queue's recent jobs.</p>
          ) : (
            <ul className="flex flex-col gap-1 text-sm" data-testid="scheduler-last-runs">
              {runs.map((j) => (
                <li key={j.jobRef.id} className="flex flex-wrap items-baseline gap-x-2">
                  <Link
                    to="/connections/$connectionId/queues/$queueId"
                    params={{ connectionId, queueId }}
                    search={{ job: j.jobRef.id }}
                    className="font-tabular text-xs wrap-anywhere hover:underline"
                  >
                    #{j.jobRef.id}
                  </Link>
                  <span
                    className={
                      j.state === "failed" ? "text-(--state-failed-text)" : "text-(--text2)"
                    }
                  >
                    {j.state} ·{" "}
                    {formatAge(Math.max(now - (j.finishedOn ?? j.processedOn ?? j.timestamp), 0))}{" "}
                    ago
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>

      <SchedulerSheet
        open={editing}
        onOpenChange={setEditing}
        connectionId={connectionId}
        queueId={queueId}
        queueName={queueName}
        scheduler={scheduler}
      />
      <ConfirmSheet
        open={confirming !== null}
        onOpenChange={(open) => {
          if (!open) setConfirming(null)
        }}
        title={
          confirming === "remove"
            ? `Remove the scheduler ${scheduler.key}?`
            : `Run ${scheduler.key} once now?`
        }
        description={
          confirming === "remove"
            ? "It stops making jobs. The jobs it already made stay."
            : `Adds one ${scheduler.name} job to ${queueName} with the scheduler's template. The schedule does not move.`
        }
        danger={confirming === "remove"}
        removed={
          confirming === "remove" ? [{ label: "The scheduler and its next planned job" }] : []
        }
        kept={confirming === "remove" ? [{ label: "The jobs it already made" }] : []}
        protectedName={locked ? queueName : undefined}
        confirmLabel={confirming === "remove" ? "Remove scheduler" : "Run once now"}
        onConfirm={() => void confirm()}
        busy={busy}
        testId="scheduler-confirm"
        confirmTestId="scheduler-confirm-action"
      />
    </div>
  )
}

function pretty(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2)
  } catch {
    return raw
  }
}

/** The inspector for `queue/key`, found among the connection's schedulers. */
export function SchedulerInspectorFor({
  connectionId,
  id,
  onClose,
}: {
  connectionId: string
  id: string
  onClose: () => void
}) {
  const { rows } = useConnectionSchedulers(connectionId)
  const row = rows.find((r) => rowId(r) === id)
  const recent = useLatestJobsOf(connectionId, row ? [row.queueId] : [])
  if (!row) return null
  return (
    <SchedulerInspector
      key={id}
      connectionId={connectionId}
      queueId={row.queueId}
      queueName={row.queueName}
      scheduler={row.scheduler}
      recentJobs={recent.get(row.queueId) ?? []}
      onClose={onClose}
    />
  )
}
