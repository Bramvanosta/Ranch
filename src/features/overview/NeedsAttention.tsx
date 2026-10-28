import { Link } from "@tanstack/react-router"
import { useJobStates } from "@/capabilities"
import { SectionHeader } from "@/components/data/SectionHeader"
import { buttonVariants } from "@/components/ui/button"
import { useFailureCauses } from "@/data/jobs/failures"
import { useCauseNotes } from "@/data/notes/queries"
import { formatCount } from "@/domain/format"
import { formatAge } from "@/domain/formatAge"
import type { WorkerCounts } from "@/domain/queueTriage"
import { inLine } from "@/domain/stateCounts"
import { AiNote, noteForCause } from "@/features/notes"
import type { FailureCause, Queue, QueueId } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

/** Causes listed; the Failed tab has the rest. */
const MAX_CAUSES = 5

/**
 * What needs the user on this connection, each with a
 * way to it: the failure causes, the paused queues, the queues
 * with waiting jobs and no worker.
 */
export function NeedsAttention({
  connectionId,
  queues,
  workers,
  now,
}: {
  connectionId: string
  queues: readonly Queue[]
  workers: WorkerCounts
  now: number
}) {
  // The refresh setting (10 s floor), one self-paced poller.
  const causes = useFailureCauses(connectionId, null, { floorMs: 10_000 })
  const states = useJobStates(connectionId)
  const waiting = (q: Queue) => inLine(q.counts, states)
  const paused = queues.filter((q) => q.isPaused)
  const workerless = queues.filter((q) => !q.isPaused && waiting(q) > 0 && workers?.get(q.id) === 0)
  const topCauses = (causes.data?.causes ?? []).slice(0, MAX_CAUSES)
  // An agent's note under the cause it explains.
  const notes = useCauseNotes({ connectionId })
  const empty = topCauses.length === 0 && paused.length === 0 && workerless.length === 0

  return (
    <section
      aria-label="Needs attention"
      className="flex flex-col gap-2"
      data-testid="needs-attention"
    >
      <SectionHeader title="Needs attention" />
      {empty ? (
        <p className="rounded-card border border-(--line) px-3 py-6 text-center text-sm text-(--text2)">
          {causes.isLoading ? "Looking" : "Nothing needs you on this connection."}
        </p>
      ) : (
        <ul className="divide-y divide-(--line) rounded-card border border-(--line)">
          {topCauses.map((cause) => (
            <Item
              key={`cause:${cause.signature}`}
              tone="failed"
              title={
                <>
                  <span className="font-tabular text-(--state-failed-text)">
                    {formatCount(cause.count)}
                  </span>{" "}
                  <span className="font-tabular break-words">
                    {cause.signature === "" ? "No failure reason recorded" : cause.signature}
                  </span>
                </>
              }
              detail={`${cause.queueNames.join(", ")} · last ${formatAge(Math.max(now - cause.lastAt, 0))} ago`}
              open={<CauseLink connectionId={connectionId} cause={cause} />}
              testId="attention-cause"
              below={(() => {
                const note = noteForCause(notes, cause)
                return note ? <AiNote note={note} /> : null
              })()}
            />
          ))}
          {paused.map((q) => (
            <Item
              key={`paused:${q.name}`}
              tone="paused"
              title={<span className="break-words">{q.name} is paused</span>}
              detail={`${formatCount(waiting(q))} waiting until it is resumed`}
              open={<QueueLink connectionId={connectionId} queue={q.id} />}
              testId="attention-paused"
            />
          ))}
          {workerless.map((q) => (
            <Item
              key={`noworker:${q.name}`}
              tone="noWorker"
              title={<span className="break-words">{q.name} has no worker</span>}
              detail={`${formatCount(waiting(q))} waiting, nobody is taking them`}
              open={<QueueLink connectionId={connectionId} queue={q.id} view="workers" />}
              testId="attention-no-worker"
            />
          ))}
        </ul>
      )}
    </section>
  )
}

const DOT = {
  failed: "bg-(--state-failed)",
  paused: "bg-(--state-delayed)",
  noWorker: "bg-(--state-delayed)",
} as const

function Item({
  tone,
  title,
  detail,
  open,
  testId,
  below,
}: {
  tone: keyof typeof DOT
  title: React.ReactNode
  detail: string
  open: React.ReactNode
  testId: string
  /** Under the line, full width: an agent's note. */
  below?: React.ReactNode
}) {
  return (
    <li className="flex flex-col gap-2 px-3 py-2" data-testid={testId}>
      <div className="flex items-start gap-3">
        <span className={cn("mt-1.5 size-2 shrink-0 rounded-full", DOT[tone])} aria-hidden="true" />
        <span className="flex min-w-0 flex-1 flex-col gap-0.5 text-sm">
          <span>{title}</span>
          <span className="text-xs break-words text-(--text2)">{detail}</span>
        </span>
        {open}
      </div>
      {below ? <div className="pl-5">{below}</div> : null}
    </li>
  )
}

const OPEN = cn(buttonVariants({ variant: "default", size: "sm" }), "shrink-0")

function CauseLink({ connectionId, cause }: { connectionId: string; cause: FailureCause }) {
  // One queue: its id is on the cause's jobs.
  const only = cause.queueNames.length === 1 ? cause.jobs[0]?.queueId : undefined
  return only ? (
    <Link
      to="/connections/$connectionId/queues/$queueId"
      params={{ connectionId, queueId: only }}
      search={{ view: "jobs", state: "failed", group: "cause", page: 0 }}
      className={OPEN}
    >
      Open
    </Link>
  ) : (
    <Link
      to="/connections/$connectionId"
      params={{ connectionId }}
      search={{ view: "failed" }}
      className={OPEN}
    >
      Open
    </Link>
  )
}

function QueueLink({
  connectionId,
  queue,
  view,
}: {
  connectionId: string
  queue: QueueId
  view?: "workers"
}) {
  return (
    <Link
      to="/connections/$connectionId/queues/$queueId"
      params={{ connectionId, queueId: queue }}
      search={view ? { view } : { view: "jobs", state: "waiting", page: 0 }}
      className={OPEN}
    >
      Open
    </Link>
  )
}
