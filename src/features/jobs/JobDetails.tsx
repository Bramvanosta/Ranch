import { Link } from "@tanstack/react-router"
import type { ReactNode } from "react"

import { formatDurationMs } from "@/features/metrics/format"
import type { Job } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { JobProgress } from "./JobProgress"

/**
 * The facts about a job that are not dates (those are the timeline): where it
 * lives, how many tries it has and has used, how it backs off, and — only
 * when they apply — priority, delay, progress, parent job, scheduler.
 *
 * Rows that do not apply are left out: a column of dashes is not information.
 * The raw options stay one click away, in the Options tab.
 */
export interface JobOptions {
  attempts?: number
  backoff?: { type?: string; delay?: number } | number
  priority?: number
}

interface JobDetailsProps {
  job: Job
  connectionId: string
  opts: JobOptions
  className?: string
}

export function JobDetails({ job, connectionId, opts, className }: JobDetailsProps) {
  const queue = job.jobRef.queue
  const maxAttempts = typeof opts.attempts === "number" ? opts.attempts : null
  const backoff = formatBackoff(opts.backoff)
  const parent = parseJobKey(job.parentKey)
  const schedulerId = /^repeat:(.+):\d+$/.exec(job.jobRef.id)?.[1] ?? null
  const delay = typeof job.delay === "number" && job.delay > 0 ? job.delay : null
  const priority = typeof opts.priority === "number" && opts.priority > 0 ? opts.priority : null

  return (
    // Two columns beside the timeline; four when the pane is narrow and the
    // details sit under it, full width.
    <div className={cn("@container", className)}>
      <dl
        className="grid grid-cols-2 content-start gap-x-6 gap-y-4 @md:grid-cols-3 @2xl:grid-cols-4"
        data-testid="job-details"
      >
        <Item label="Queue">
          <Link
            to="/connections/$connectionId/queues/$queueName"
            params={{ connectionId, queueName: queue.name }}
            className="font-mono text-xs hover:underline"
            title={`${queue.prefix}:${queue.name}`}
          >
            {queue.name}
          </Link>
        </Item>
        <Item label="Attempts" testId="job-details-attempts">
          <span className="flex items-center gap-2">
            <span className="font-mono text-xs tabular-nums">
              {maxAttempts === null ? job.attemptsMade : `${job.attemptsMade} of ${maxAttempts}`}
            </span>
            {maxAttempts !== null && maxAttempts > 1 && maxAttempts <= 10 ? (
              <AttemptPips
                made={job.attemptsMade}
                max={maxAttempts}
                failed={job.state === "failed"}
              />
            ) : null}
          </span>
        </Item>
        {backoff ? (
          <Item label="Backoff">
            <span className="font-mono text-xs">{backoff}</span>
          </Item>
        ) : null}
        {priority !== null ? (
          <Item label="Priority">
            <span className="font-mono text-xs tabular-nums" title="1 is the highest priority">
              {priority}
            </span>
          </Item>
        ) : null}
        {delay !== null ? (
          <Item label="Delay">
            <span className="font-mono text-xs tabular-nums">{formatConfiguredDelay(delay)}</span>
          </Item>
        ) : null}
        {job.progress !== null ? (
          <Item label="Progress">
            <JobProgress progress={job.progress} />
          </Item>
        ) : null}
        {parent ? (
          <Item label="Parent job" wide>
            <Link
              to="/connections/$connectionId/queues/$queueName/$jobId"
              params={{ connectionId, queueName: parent.queue, jobId: parent.id }}
              search={{ tab: "flow" }}
              className="font-mono text-xs hover:underline"
              title={job.parentKey ?? undefined}
              data-testid="job-details-parent"
            >
              {parent.queue} #{parent.id}
            </Link>
          </Item>
        ) : null}
        {schedulerId ? (
          <Item label="Scheduler" wide>
            <Link
              to="/connections/$connectionId/queues/$queueName"
              params={{ connectionId, queueName: queue.name }}
              search={{ view: "cron" }}
              className="font-mono text-xs hover:underline"
              data-testid="job-details-scheduler"
            >
              {schedulerId}
            </Link>
          </Item>
        ) : null}
      </dl>
    </div>
  )
}

function Item({
  label,
  children,
  wide,
  testId,
}: {
  label: string
  children: ReactNode
  wide?: boolean
  testId?: string
}) {
  return (
    <div className={cn("min-w-0", wide && "col-span-2")} data-testid={testId}>
      <dt className="text-2xs font-semibold uppercase tracking-wide text-(--text2)">{label}</dt>
      <dd className="mt-1 min-w-0 truncate text-sm">{children}</dd>
    </div>
  )
}

/** One dot per allowed attempt: used ones filled (red once the job has failed for good). */
function AttemptPips({ made, max, failed }: { made: number; max: number; failed: boolean }) {
  return (
    <span className="flex items-center gap-1" aria-hidden="true">
      {Array.from({ length: max }, (_, i) => (
        <span
          // biome-ignore lint/suspicious/noArrayIndexKey: fixed-length, order-only list
          key={i}
          className={cn(
            "size-1.5 rounded-full",
            i < made
              ? failed
                ? "bg-[var(--state-failed)]"
                : "bg-(--text)/70"
              : "border border-(--text2)/50",
          )}
        />
      ))}
    </span>
  )
}

/** A configured delay: `1 s`, `5 min`, `2 h` when it is round — `1.5 s` otherwise. */
export function formatConfiguredDelay(ms: number): string {
  if (ms >= 3_600_000 && ms % 3_600_000 === 0) return `${ms / 3_600_000} h`
  if (ms >= 60_000 && ms % 60_000 === 0) return `${ms / 60_000} min`
  if (ms >= 1_000 && ms % 1_000 === 0) return `${ms / 1_000} s`
  return formatDurationMs(ms)
}

export function formatBackoff(b: unknown): string | null {
  if (b == null) return null
  if (typeof b === "number") return b > 0 ? `fixed · ${formatConfiguredDelay(b)}` : null
  if (typeof b === "object") {
    const obj = b as { type?: string; delay?: number }
    const delay =
      typeof obj.delay === "number" && obj.delay > 0 ? formatConfiguredDelay(obj.delay) : null
    if (obj.type && delay) return `${obj.type} · ${delay}`
    return obj.type ?? delay
  }
  return null
}

/** `prefix:queue:id` — the queue name may itself contain colons, the id may not. */
export function parseJobKey(key: string | null): { queue: string; id: string } | null {
  if (!key) return null
  const parts = key.split(":")
  if (parts.length < 3) return null
  const id = parts[parts.length - 1] ?? ""
  const queue = parts.slice(1, -1).join(":")
  return id && queue ? { queue, id } : null
}
