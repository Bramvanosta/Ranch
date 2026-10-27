import { useQuery, useQueryClient } from "@tanstack/react-query"
import { ChevronRightIcon } from "lucide-react"
import { type ReactNode, useState } from "react"
import { toast } from "sonner"

import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { AskAiMenu } from "@/features/askai/AskAiMenu"
import { useConnection } from "@/features/connections/useConnection"
import { StaleNotice } from "@/features/freshness/StaleNotice"
import { AiNote } from "@/features/notes/AiNote"
import { noteForCause, useCauseNotes } from "@/features/notes/useCauseNotes"
import { withOneShotUnlock } from "@/features/safety/oneShotUnlock"
import { useConnectionLockState } from "@/features/safety/useConnectionLockState"
import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import {
  type CauseNote,
  commands,
  type FailureCause,
  type Job,
  type QueueRef,
} from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { formatAge } from "./formatAge"
import { firstLine } from "./jobColumns"
import { useBulkRemove, useBulkRetry } from "./useBulkAction"

/** Jobs listed under an open cause; the rest are counted. */
const JOBS_PER_CAUSE = 50

export const FAILURE_CAUSES_QUERY_KEY = (connectionId: string, queueName: string) =>
  ["failureCauses", connectionId, queueName] as const

/**
 * The Failed tab grouped by what went wrong: one
 * folding group per cause (the failure reason with its values normalised,
 * `bullmq_schema::failed::failure_signature`), most frequent first. The
 * open group lists its jobs; each group retries or removes its jobs through
 * sheets.
 */
export function FailureCausesView({
  connectionId,
  queueRef,
  toggle,
  onOpenJob,
}: {
  connectionId: string
  queueRef: QueueRef
  /** The List / By cause switch, shown at the head of the view. */
  toggle: ReactNode
  onOpenJob: (job: Job) => void
}) {
  // The refresh setting (5 s floor), one self-paced poller.
  useSharedPoll(FAILURE_CAUSES_QUERY_KEY(connectionId, queueRef.name), useRefetchInterval(5_000))
  const query = useQuery({
    queryKey: FAILURE_CAUSES_QUERY_KEY(connectionId, queueRef.name),
    queryFn: async () => {
      const r = await commands.listFailureCauses({ connectionId, queue: queueRef.name })
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    refetchInterval: false,
  })
  const [open, setOpen] = useState<string | null>(null)
  const [now] = useState(() => Date.now())
  // agents' notes on these causes.
  const notes = useCauseNotes({ connectionId })
  const data = query.data
  const causes = data?.causes ?? []

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3" data-testid="failure-causes">
      <div className="flex flex-wrap items-center gap-3">
        {toggle}
        <span className="text-sm text-(--text2)" data-testid="failure-causes-summary">
          {data
            ? `${causes.length.toLocaleString()} ${causes.length === 1 ? "cause" : "causes"} · ${data.scanned.toLocaleString()} failed ${data.scanned === 1 ? "job" : "jobs"}`
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
              note={noteForCause(notes, cause, queueRef.name)}
              now={now}
              open={open === cause.signature}
              onToggle={() => setOpen((o) => (o === cause.signature ? null : cause.signature))}
              connectionId={connectionId}
              queueRef={queueRef}
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
  queueRef,
  onOpenJob,
}: {
  cause: FailureCause
  note?: CauseNote
  now: number
  open: boolean
  onToggle: () => void
  connectionId: string
  queueRef: QueueRef
  onOpenJob: (job: Job) => void
}) {
  const [confirming, setConfirming] = useState<"retry" | "remove" | null>(null)
  const connection = useConnection(connectionId)
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
              target={{ kind: "cause", connection, cause, queueName: queueRef.name }}
              testId="failure-cause-ask-ai"
            />
          ) : null}
          <button
            type="button"
            onClick={() => setConfirming("retry")}
            className="h-control-sm rounded-control px-2 text-xs hover:bg-(--surf) focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)/60"
            data-testid="failure-cause-retry"
          >
            Retry {cause.count.toLocaleString()}
          </button>
          <button
            type="button"
            onClick={() => setConfirming("remove")}
            className="h-control-sm rounded-control px-2 text-xs text-(--state-failed-text) hover:bg-(--surf) focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)/60"
            data-testid="failure-cause-remove"
          >
            Remove {cause.count.toLocaleString()}
          </button>
        </div>
      </div>
      {note ? (
        <AiNote
          note={note}
          retryCount={cause.count}
          onRetry={() => setConfirming("retry")}
          className="mx-3 mb-2 ml-[5.25rem]"
        />
      ) : null}
      {open ? (
        <div id={panelId}>
          <CauseJobs
            connectionId={connectionId}
            queueRef={queueRef}
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
        queueRef={queueRef}
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
  queueRef,
  cause,
  now,
  onOpenJob,
}: {
  connectionId: string
  queueRef: QueueRef
  cause: FailureCause
  now: number
  onOpenJob: (job: Job) => void
}) {
  const query = useQuery({
    queryKey: ["failureCauseJobs", connectionId, queueRef.name, cause.signature],
    queryFn: async () => {
      const r = await commands.listFailedJobs({
        connectionId,
        offset: 0,
        limit: JOBS_PER_CAUSE,
        queue: queueRef.name,
        name: null,
        reason: null,
        signature: cause.signature,
      })
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
  })
  if (query.isLoading) {
    return <p className="px-12 pb-3 text-xs text-(--text2)">Loading the jobs of this cause</p>
  }
  const page = query.data
  if (!page) return null
  return (
    <div className="pb-2" data-testid="failure-cause-jobs">
      <ul>
        {page.jobs.map((job) => (
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
      {page.total > page.jobs.length ? (
        <p className="px-12 pt-1 text-xs text-(--text2)">
          {page.jobs.length.toLocaleString()} of {page.total.toLocaleString()} shown, newest first.
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
  queueRef,
}: {
  action: "retry" | "remove" | null
  onClose: () => void
  cause: FailureCause
  label: string
  jobs: string
  connectionId: string
  queueRef: QueueRef
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
    const input = { connectionId, queueRef, jobIds: cause.jobs.map((j) => j.id) }
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
      qc.invalidateQueries({ queryKey: FAILURE_CAUSES_QUERY_KEY(connectionId, queueRef.name) })
      qc.invalidateQueries({ queryKey: ["failureCauseJobs", connectionId, queueRef.name] })
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
          label: removing ? `Every other job of ${queueRef.name}` : "Their data, attempts and logs",
        },
      ]}
      protectedName={locked ? queueRef.name : undefined}
      confirmLabel={`${removing ? "Remove" : "Retry"} ${jobs}`}
      onConfirm={() => void run()}
      busy={busy}
      testId="failure-cause-sheet"
      confirmTestId="failure-cause-confirm"
    />
  )
}
