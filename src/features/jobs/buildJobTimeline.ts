import { formatDurationMs } from "@/domain/format"
import { formatAge } from "@/domain/formatAge"
import { waitingStates } from "@/domain/jobStates"
import type { Job, JobState, JobStateInfo } from "@/lib/tauri/bindings"

/**
 * The life of a job as a few dated steps, what the job page draws as a
 * timeline: added → (due) → started → completed / failed, with what is still
 * to come as a pending step ("waiting for a worker", "running").
 *
 * Pure: `now` is a parameter, so a running job ticks by re-calling it.
 *
 * What BullMQ really stores, and what that allows:
 *   - `timestamp`   when the job was added;
 *   - `processedOn` when the **last** attempt started (each attempt
 *     overwrites it), hence "Started · attempt 3";
 *   - `finishedOn`  when it completed or failed for good;
 *   - `delay`       the initial delay, but a retry with backoff rewrites it
 *     and is due from *that* moment, which the hash does not keep. The due
 *     time is therefore only shown for a job that never ran.
 */
export type TimelineTone = "done" | "completed" | "failed" | "active" | "pending"

export interface TimelineStep {
  key: "added" | "due" | "started" | "finished" | "pending"
  label: string
  /** Epoch ms; `null` for a step that has not happened (or whose time is unknown). */
  at: number | null
  tone: TimelineTone
  /** The gap that led here, or what is being waited for. */
  note: string | null
}

/** BullMQ's wording of a parent's wait: no other backend keeps the state. */
const OWN_STEPS: Partial<Record<JobState, string>> = {
  waitingChildren: "Waiting for its children",
}

/**
 * What a job that is not running waits for: a worker in the states the
 * backend describes as waiting, a resume when paused.
 */
function pendingLabel(state: JobState, states: ReadonlyArray<JobStateInfo>): string | undefined {
  if (state === "paused") return "Queue is paused"
  if (waitingStates(states).includes(state)) return "Waiting for a worker"
  return OWN_STEPS[state]
}

export function buildJobTimeline(
  job: Job,
  now: number,
  states: ReadonlyArray<JobStateInfo>,
): TimelineStep[] {
  const steps: TimelineStep[] = [
    {
      key: "added",
      label: "Added",
      at: job.timestamp,
      tone: "done",
      note: `${formatAge(now - job.timestamp)} ago`,
    },
  ]

  const delay = typeof job.delay === "number" && job.delay > 0 ? job.delay : 0
  const neverRan = job.processedOn === null && job.attemptsMade === 0

  if (job.state === "delayed") {
    if (neverRan && delay > 0) {
      const due = job.timestamp + delay
      steps.push({
        key: "due",
        label: "Due",
        at: due,
        tone: "pending",
        note:
          due >= now
            ? `in ${formatDurationMs(due - now)}`
            : `${formatDurationMs(now - due)} ago, no worker took it yet`,
      })
    } else {
      steps.push({
        key: "pending",
        label: "Retry scheduled",
        at: null,
        tone: "pending",
        note: delay > 0 ? `after a ${formatDurationMs(delay)} backoff` : null,
      })
    }
    return steps
  }

  if (job.processedOn === null) {
    const label = pendingLabel(job.state, states)
    if (label) steps.push({ key: "pending", label, at: null, tone: "pending", note: null })
    // A finished job without `processedOn` (older BullMQ, trimmed hash): the
    // end is still worth showing.
    if (job.finishedOn !== null) steps.push(finishedStep(job, null))
    return steps
  }

  // The attempt `processedOn` belongs to: an attempt is counted when it ends.
  const attempt = job.state === "active" ? job.attemptsMade + 1 : Math.max(job.attemptsMade, 1)
  const sinceAdded = job.processedOn - job.timestamp
  steps.push({
    key: "started",
    label: attempt > 1 ? `Started · attempt ${attempt}` : "Started",
    at: job.processedOn,
    tone: "done",
    note:
      sinceAdded < 0
        ? null
        : attempt > 1
          ? `${formatDurationMs(sinceAdded)} after it was added`
          : `waited ${sinceAdded === 0 ? "0 ms" : formatDurationMs(sinceAdded)}`,
  })

  if (job.finishedOn !== null) {
    steps.push(finishedStep(job, job.processedOn))
  } else if (job.state === "active") {
    steps.push({
      key: "pending",
      label: "Running",
      at: null,
      tone: "active",
      note: `for ${formatDurationMs(Math.max(now - job.processedOn, 1))}`,
    })
  } else {
    // Ran, did not finish, is not running: between two attempts.
    const label = pendingLabel(job.state, states)
    if (label) {
      steps.push({ key: "pending", label, at: null, tone: "pending", note: "to be retried" })
    }
  }
  return steps
}

function finishedStep(job: Job, processedOn: number | null): TimelineStep {
  const finishedOn = job.finishedOn as number
  const ran = processedOn !== null && finishedOn >= processedOn ? finishedOn - processedOn : null
  return {
    key: "finished",
    label: job.state === "failed" ? "Failed" : job.state === "completed" ? "Completed" : "Finished",
    at: finishedOn,
    tone: job.state === "failed" ? "failed" : job.state === "completed" ? "completed" : "done",
    note: ran === null ? null : `ran ${ran === 0 ? "0 ms" : formatDurationMs(ran)}`,
  }
}

const pad = (n: number) => String(n).padStart(2, "0")

/** `YYYY-MM-DD HH:MM:SS`, local, what a log line next to it would show. */
export function formatLocalDateTime(ms: number): string {
  const d = new Date(ms)
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${formatLocalTime(ms)}`
}

export function formatLocalTime(ms: number): string {
  const d = new Date(ms)
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

/** The date is written once: later steps on the same day show the time only. */
export function formatStepTime(ms: number, firstMs: number): string {
  const a = new Date(ms)
  const b = new Date(firstMs)
  const sameDay =
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  return sameDay && ms !== firstMs ? formatLocalTime(ms) : formatLocalDateTime(ms)
}
