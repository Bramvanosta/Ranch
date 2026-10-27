import type { TraceRow } from "@/components/data/TraceBar"
import { formatDurationMs } from "@/features/metrics/format"
import type { Job } from "@/lib/tauri/bindings"

import { buildJobTimeline } from "./buildJobTimeline"
import type { JobOptions } from "./JobDetails"

/**
 * A job's life as rows on one time axis: the wait for a
 * worker, each attempt, each backoff between two attempts, and what is still
 * to come.
 *
 * Pure: `now` is a parameter, so a running job ticks by re-calling it.
 *
 * BullMQ keeps the times of the **last** attempt only (`processedOn`,
 * `finishedOn`). The earlier attempts are placed backwards from it with the
 * job's backoff setting and the last attempt's duration, and are marked
 * `estimated` — the trace says so rather than draw them as facts.
 */
export interface JobTrace {
  rows: TraceRow[]
  /** The axis: when the job was added… */
  start: number
  /** …to when it finished, or now. */
  end: number
  /** "Added → failed in 1 h 12 min · 5.18 s of actual work". */
  summary: string
  /** Some rows are placed from the backoff setting, not from stored times. */
  estimated: boolean
}

export function buildJobTrace(job: Job, opts: JobOptions, now: number): JobTrace {
  const added = job.timestamp
  const rows: TraceRow[] = []
  const finished = job.state === "failed" || job.state === "completed"
  const running = job.state === "active"

  if (job.processedOn === null) {
    // Never ran: one pending row, from the timeline's wording.
    const pending = buildJobTimeline(job, now).find((s) => s.key === "pending" || s.key === "due")
    rows.push({
      key: "waiting",
      label: "Waiting",
      start: added,
      end: now,
      kind: "pending",
      tone: "neutral",
      detail: pending?.label ?? "Waiting for a worker",
    })
    return {
      rows,
      start: added,
      end: Math.max(now, added + 1),
      summary: `Added ${formatDurationMs(Math.max(now - added, 0))} ago · not run yet`,
      estimated: false,
    }
  }

  const lastStart = job.processedOn
  // How many attempts the trace shows, the last one included.
  const attempts = running ? job.attemptsMade + 1 : Math.max(job.attemptsMade, 1)
  const lastEnd = finished ? (job.finishedOn ?? lastStart) : running ? now : null
  const lastDuration = lastEnd !== null ? Math.max(lastEnd - lastStart, 0) : 0

  // Walk back from the last attempt: before attempt k+1, backoff k, before
  // that attempt k, as long as the last one.
  const starts: number[] = [lastStart]
  const backoffs: (number | null)[] = []
  for (let k = attempts - 1; k >= 1; k--) {
    const b = backoffMs(opts.backoff, k)
    backoffs.unshift(b)
    const nextStart = starts[0] ?? lastStart
    starts.unshift(Math.max(nextStart - (b ?? 0) - lastDuration, added))
  }
  const firstStart = starts[0] ?? lastStart
  const estimated = attempts > 1

  rows.push({
    key: "waiting",
    label: "Waiting",
    start: added,
    end: firstStart,
    kind: "wait",
    tone: "neutral",
    detail: formatDurationMs(Math.max(firstStart - added, 0)),
    estimated,
  })
  for (let i = 0; i < attempts; i++) {
    const start = starts[i] ?? lastStart
    const last = i === attempts - 1
    if (!last) {
      rows.push({
        key: `attempt-${i + 1}`,
        label: `Attempt ${i + 1}`,
        start,
        end: start + lastDuration,
        kind: "work",
        tone: "failed",
        detail: "failed",
        estimated: true,
      })
      const b = backoffs[i] ?? null
      const bStart = start + lastDuration
      rows.push({
        key: `backoff-${i + 1}`,
        label: "Backoff",
        start: bStart,
        end: starts[i + 1] ?? bStart,
        kind: "backoff",
        tone: "neutral",
        detail: b !== null ? formatDurationMs(b) : "custom",
        estimated: true,
      })
      continue
    }
    rows.push({
      key: `attempt-${i + 1}`,
      label: `Attempt ${i + 1}`,
      start,
      end: lastEnd ?? start,
      kind: "work",
      tone: running ? "running" : job.state === "completed" ? "completed" : "failed",
      detail: running
        ? `running · ${formatDurationMs(lastDuration)}`
        : lastEnd !== null
          ? `${job.state === "completed" ? "completed" : "failed"} · ${formatDurationMs(lastDuration)}`
          : "failed",
    })
  }

  // Between two attempts (BullMQ will try again), or waiting to run again.
  if (!finished && !running) {
    const pending = buildJobTimeline(job, now).find((s) => s.key === "pending")
    const from = lastEnd ?? lastStart
    rows.push({
      key: "pending",
      label: "Next",
      start: from,
      end: Math.max(now, from),
      kind: "pending",
      tone: "neutral",
      detail: pending?.label ?? "Retry scheduled",
    })
  }

  const end = Math.max(finished ? (job.finishedOn ?? lastStart) : now, added + 1)
  const work = lastDuration * attempts
  const outcome = finished ? (job.state === "completed" ? "completed" : "failed") : null
  const total = formatDurationMs(Math.max(end - added, 0))
  const workText = `${estimated ? "≈ " : ""}${formatDurationMs(work)} of actual work`
  const summary = outcome
    ? `Added → ${outcome} in ${total} · ${workText}`
    : `Added ${total} ago · ${workText} so far`
  return { rows, start: added, end, summary, estimated }
}

/**
 * The delay BullMQ waits after failed attempt `k` (1-based): a number or
 * `{type: "fixed", delay}` waits `delay`; `{type: "exponential", delay}`
 * waits `delay × 2^(k−1)`. A custom strategy is the worker's: unknown.
 */
export function backoffMs(backoff: JobOptions["backoff"], k: number): number | null {
  if (typeof backoff === "number") return backoff
  if (!backoff || typeof backoff.delay !== "number") return backoff ? null : 0
  if (backoff.type === "exponential") return backoff.delay * 2 ** (k - 1)
  if (backoff.type === undefined || backoff.type === "fixed") return backoff.delay
  return null
}
