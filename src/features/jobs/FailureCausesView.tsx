import { useQueryClient } from "@tanstack/react-query"
import { ChevronRightIcon } from "lucide-react"
import { type ReactNode, useState } from "react"
import { toast } from "sonner"

import { useCapabilities } from "@/capabilities"
import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { useConnection } from "@/data/connections/queries"
import { useBulkRemove, useBulkRetry } from "@/data/jobs/bulkMutations"
import { useCauseJobs, useFailureCauses } from "@/data/jobs/failures"
import { jobKeys } from "@/data/jobs/keys"
import { useCauseNotes } from "@/data/notes/queries"
import { withOneShotUnlock } from "@/data/safety/oneShotUnlock"
import { useConnectionLockState } from "@/data/safety/queries"
import { formatAge } from "@/domain/formatAge"
import { AskAiMenu } from "@/features/askai"
import { StaleNotice } from "@/features/freshness"
import { AiNote, noteForCause } from "@/features/notes"
import type { CauseNote, FailureCause, Job, QueueId } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { firstLine } from "./jobColumns"

/** Jobs listed under an open cause; the rest are counted. */
const JOBS_PER_CAUSE = 50

/**
 * A cause narrowed to the jobs of one queue. Causes are read by queue name,
 * and two prefixes of a connection can hold a queue of the same name
 * (`bull:emails`, `app:emails`): only this queue's jobs are counted, listed
 * and acted on. `null` when none of the cause's jobs are in it.
 */
export function causeInQueue(cause: FailureCause, queueId: QueueId): FailureCause | null {
  const jobs = cause.jobs.filter((j) => j.queueId === queueId)
  if (jobs.length === 0) return null
  return jobs.length === cause.jobs.length ? cause : { ...cause, jobs, count: jobs.length }
}

/**
 * The Failed tab grouped by what went wrong: one
 * folding group per cause (the failure reason with its values normalised,
 * `bullmq_schema::failed::failure_signature`), most frequent first. The
 * open group lists its jobs; each group retries or removes its jobs through
 * sheets.
 */
export function FailureCausesView({
  connectionId,
  queueId,
  queueName,
  toggle,
  onOpenJob,
}: {
  connectionId: string
  queueId: QueueId
  queueName: string
  /** The List / By cause switch, shown at the head of the view. */
  toggle: ReactNode
  onOpenJob: (job: Job) => void
}) {
  // The refresh setting (5 s floor), one self-paced poller.
  const query = useFailureCauses(connectionId, queueName, { floorMs: 5_000 })
  const [open, setOpen] = useState<string | null>(null)
  const [now] = useState(() => Date.now())
  // agents' notes on these causes.
  const notes = useCauseNotes({ connectionId })
  const data = query.data
  const causes = (data?.causes ?? []).flatMap((c) => causeInQueue(c, queueId) ?? [])
  const scanned = causes.reduce((n, c) => n + c.count, 0)

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3" data-testid="failure-causes">
      <div className="flex flex-wrap items-center gap-3">
        {toggle}
        <span className="text-sm text-(--text2)" data-testid="failure-causes-summary">
          {data
            ? `${causes.length.toLocaleString()} ${causes.length === 1 ? "cause" : "causes"} · ${scanned.toLocaleString()} failed ${scanned === 1 ? "job" : "jobs"}`
            : "Counting"}
        </span>
        {data?.scanCapped ? (
          <span className="text-xs text-(--text2)">
            Grouped from the newest {data.scanned.toLocaleString()} of{" "}
            {data.totalFailed.toLocaleString()} failed jobs.
          </span>
        ) : null}
      </div>
      {query.isError && data ? (
        <StaleNotice error={query.error} updatedAt={query.dataUpdatedAt} what="the causes" />
      ) : null}
      {query.isError && !data ? (
        <p className="text-sm text-(--state-failed-text)" role="alert">
          {query.error instanceof Error ? query.error.message : "Could not group the failures."}
        </p>
      ) : data && causes.length === 0 ? (
        <p className="py-8 text-center text-sm text-(--text2)">Nothing failed in this queue.</p>
      ) : (
        <ul className="min-h-0 flex-1 overflow-auto rounded-card border border-(--line)">
          {causes.map((cause) => (
            <CauseGroup
              key={cause.signature}
              cause={cause}
              note={noteForCause(notes, cause, queueId)}
              now={now}
              open={open === cause.signature}
              onToggle={() => setOpen((o) => (o === cause.signature ? null : cause.signature))}
              connectionId={connectionId}
              queueId={queueId}
              queueName={queueName}
              onOpenJob={onOpenJob}
            />
          ))}
        </ul>
      )}
    </div>
  )
}

function CauseGroup({
  cause,
  note,
  now,
  open,
  onToggle,
  connectionId,
  queueId,
  queueName,
  onOpenJob,
}: {
  cause: FailureCause
  note?: CauseNote
  now: number
  open: boolean
  onToggle: () => void
  connectionId: string
  queueId: QueueId
  queueName: string
  onOpenJob: (job: Job) => void
}) {
  const [confirming, setConfirming] = useState<"retry" | "remove" | null>(null)
  const connection = useConnection(connectionId)
  // Retrying or removing a cause's jobs is one bulk action.
  const bulk = useCapabilities(connectionId).can("jobActions.bulk")
  const label = cause.signature === "" ? "No failure reason recorded" : cause.signature
  const panelId = `cause-${cause.signature.length}-${cause.count}-${cause.lastAt}`
  const jobs = cause.count === 1 ? "1 job" : `${cause.count.toLocaleString()} jobs`

  return (
    <li className="border-b border-(--line) last:border-b-0" data-testid="failure-cause">
      <div className="flex items-start gap-3 px-3 py-2 hover:bg-(--surf2)">
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={open}
          aria-controls={panelId}
          className="flex min-w-0 flex-1 items-start gap-3 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)/60"
          data-testid="failure-cause-toggle"
        >
          <ChevronRightIcon
            className={cn(
              "mt-0.5 size-4 shrink-0 text-(--text2) transition-transform",
              open && "rotate-90",
            )}
            aria-hidden="true"
          />
          <span
            className="w-14 shrink-0 text-right font-tabular text-sm text-(--state-failed-text)"
            data-testid="failure-cause-count"
          >
            {cause.count.toLocaleString()}
          </span>
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span
              className="font-tabular text-sm break-words"
              data-testid="failure-cause-signature"
            >
              {label}
            </span>
            <span className="text-xs text-(--text2)">
              {cause.jobNames.join(", ")} · last {formatAge(Math.max(now - cause.lastAt, 0))} ago
            </span>
          </span>
          <Trend values={cause.trend} />
        </button>
        <div className="flex shrink-0 items-center gap-1">
          {connection ? (
            <AskAiMenu
              target={{ kind: "cause", connection, cause }}
              testId="failure-cause-ask-ai"
            />
          ) : null}
          {bulk ? (
            <>
              <button
                type="button"
                onClick={() => setConfirming("retry")}
                // A floor on the width keeps the buttons of every cause in one
                // column, whatever the count's digits.
                className="h-control-sm min-w-[5.5rem] rounded-control px-2 text-xs hover:bg-(--surf) focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)/60"
                data-testid="failure-cause-retry"
              >
                Retry {cause.count.toLocaleString()}
              </button>
              <button
                type="button"
                onClick={() => setConfirming("remove")}
                className="h-control-sm min-w-[6.5rem] rounded-control px-2 text-xs text-(--state-failed-text) hover:bg-(--surf) focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)/60"
                data-testid="failure-cause-remove"
              >
                Remove {cause.count.toLocaleString()}
              </button>
            </>
          ) : null}
        </div>
      </div>
      {note ? (
        <AiNote
          note={note}
          retryCount={cause.count}
          onRetry={bulk ? () => setConfirming("retry") : undefined}
          className="mx-3 mb-2 ml-[5.25rem]"
        />
      ) : null}
      {open ? (
        <div id={panelId}>
          <CauseJobs
            connectionId={connectionId}
            queueId={queueId}
            queueName={queueName}
            cause={cause}
            now={now}
            onOpenJob={onOpenJob}
          />
        </div>
      ) : null}
      <CauseSheet
        action={confirming}
        onClose={() => setConfirming(null)}
        cause={cause}
        label={label}
        jobs={jobs}
        connectionId={connectionId}
        queueId={queueId}
        queueName={queueName}
      />
    </li>
  )
}

/** Failures per hour over the last 24 hours, as small bars. */
function Trend({ values }: { values: number[] }) {
  const max = Math.max(...values, 1)
  const total = values.reduce((a, b) => a + b, 0)
  return (
    <span
      className="mt-0.5 flex h-4 shrink-0 items-end gap-px"
      role="img"
      aria-label={`${total} in the last 24 hours`}
    >
      {values.map((v, i) => (
        <span
          // biome-ignore lint/suspicious/noArrayIndexKey: one bar per hour, in order.
          key={i}
          className={cn("w-0.5 rounded-t-sm", v > 0 ? "bg-(--danger)/70" : "bg-(--line)")}
          style={{ height: `${v > 0 ? Math.max((v / max) * 100, 20) : 10}%` }}
        />
      ))}
    </span>
  )
}

function CauseJobs({
  connectionId,
  queueId,
  queueName,
  cause,
  now,
  onOpenJob,
}: {
  connectionId: string
  queueId: QueueId
  queueName: string
  cause: FailureCause
  now: number
  onOpenJob: (job: Job) => void
}) {
  const query = useCauseJobs(connectionId, queueName, cause.signature, JOBS_PER_CAUSE)
  if (query.isLoading) {
    return <p className="px-12 pb-3 text-xs text-(--text2)">Loading the jobs of this cause</p>
  }
  const page = query.data
  if (!page) return null
  // Read by queue name: a same-named queue under another prefix is left out.
  const jobs = page.jobs.filter((job) => job.jobRef.queueId === queueId)
  return (
    <div className="pb-2" data-testid="failure-cause-jobs">
      <ul>
        {jobs.map((job) => (
          <li key={job.jobRef.id}>
            <button
              type="button"
              onClick={() => onOpenJob(job)}
              className="flex w-full items-start gap-3 py-1 pr-3 pl-12 text-left text-sm hover:bg-(--surf2) focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-(--accent)/60"
              data-testid="failure-cause-job"
            >
              <span className="w-20 shrink-0 font-tabular text-(--text2)">#{job.jobRef.id}</span>
              <span className="w-40 shrink-0 break-words">{job.name}</span>
              <span className="min-w-0 flex-1 font-tabular text-xs break-words text-(--state-failed-text)">
                {firstLine(job.failedReason ?? "")}
              </span>
              <span className="shrink-0 text-xs text-(--text2)">
                {formatAge(Math.max(now - (job.finishedOn ?? job.timestamp), 0))} ago
              </span>
            </button>
          </li>
        ))}
      </ul>
      {cause.count > jobs.length ? (
        <p className="px-12 pt-1 text-xs text-(--text2)">
          {jobs.length.toLocaleString()} of {cause.count.toLocaleString()} shown, newest first.
        </p>
      ) : null}
    </div>
  )
}

/** Sheet for "Retry N" and "Remove N" of one cause. */
function CauseSheet({
  action,
  onClose,
  cause,
  label,
  jobs,
  connectionId,
  queueId,
  queueName,
}: {
  action: "retry" | "remove" | null
  onClose: () => void
  cause: FailureCause
  label: string
  jobs: string
  connectionId: string
  queueId: QueueId
  queueName: string
}) {
  const qc = useQueryClient()
  const retry = useBulkRetry()
  const remove = useBulkRemove()
  const { data: lockState } = useConnectionLockState(connectionId)
  const locked = lockState === "locked"
  const [busy, setBusy] = useState(false)
  const removing = action === "remove"

  const run = async () => {
    if (!action) return
    setBusy(true)
    const input = { connectionId, queueId, jobIds: cause.jobs.map((j) => j.id) }
    const send = () => (removing ? remove : retry).mutateAsync(input)
    try {
      const result = await (locked ? withOneShotUnlock(connectionId, send) : send())
      const done = result.succeeded.toLocaleString()
      const failed = result.failed.length
      toast.success(
        `${removing ? "Removed" : "Retried"} ${done} ${result.succeeded === 1 ? "job" : "jobs"}${failed > 0 ? ` · ${failed} could not be` : ""}`,
      )
      onClose()
    } catch (err) {
      toast.error(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
      qc.invalidateQueries({ queryKey: jobKeys.failureCauseList(connectionId, queueName) })
      qc.invalidateQueries({ queryKey: jobKeys.causeJobsOf(connectionId, queueName) })
    }
  }

  return (
    <ConfirmSheet
      open={action !== null}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      title={`${removing ? "Remove" : "Retry"} ${jobs} of this cause?`}
      description={<span className="font-tabular break-words">{label}</span>}
      danger={removing}
      removed={[
        {
          label: removing ? "Failed jobs of this cause" : "Failed jobs moved back to waiting",
          count: cause.count,
        },
      ]}
      kept={[
        {
          label: removing ? `Every other job of ${queueName}` : "Their data, attempts and logs",
        },
      ]}
      protectedName={locked ? queueName : undefined}
      confirmLabel={`${removing ? "Remove" : "Retry"} ${jobs}`}
      onConfirm={() => void run()}
      busy={busy}
      testId="failure-cause-sheet"
      confirmTestId="failure-cause-confirm"
    />
  )
}
