import { notStartedStates, stateLabel, stateList } from "@/domain/jobStates"
import { agentLabel } from "@/features/mcp/agentLabel"
import type { JobStateInfo, PendingApproval } from "@/lib/tauri/bindings"

/**
 * Turn a pending approval into what the human reads (pure,
 * unit-tested). Everything that comes from the agent or from Redis is data to
 * display, never markup or instructions.
 */

export interface JobPreview {
  kind: "job"
  queue?: string
  jobId?: string
  name?: string
  state?: string
  attemptsMade?: number
  failedReason?: string | null
}

export interface BulkPreview {
  kind: "bulk"
  queue?: string
  count: number
  sampleNames?: string[]
}

/** `add_job`: everything the agent wants to enqueue. */
export interface NewJobPreview {
  kind: "newJob"
  queue?: string
  name?: string
  /** Only what the agent set: `attempts`, `delay`, `priority`, `lifo`. */
  options?: Record<string, unknown>
  /** The payload: data to display, never to interpret. */
  data?: unknown
}

/** `drain_queue`, `clean_queue`, `bulk_retry_jobs { allFailed }`. */
export interface MaintenanceApprovalPreview {
  kind: "maintenance"
  queue?: string
  operation?:
    | { kind: "drain"; includeDelayed?: boolean }
    | { kind: "clean"; state?: string; olderThanMs?: number }
    | { kind: "retryAllFailed" }
  /** Jobs the operation touches, counted when the agent asked. */
  count: number
  sparedSchedulerJobs?: number
}

/** `upsert_scheduler` / `remove_scheduler`. */
export interface SchedulerApprovalPreview {
  kind: "schedulerChange" | "schedulerRemoval"
  queue?: string
  schedulerId?: string
  jobName?: string
  /** `upsert_scheduler`: the schedule as the agent asked for it. */
  schedule?:
    | { kind: "pattern"; pattern?: string; tz?: string | null }
    | { kind: "every"; everyMs?: number }
  /** `remove_scheduler`: the scheduler as it is now. */
  pattern?: string | null
  tz?: string | null
  everyMs?: number | null
  nextRunMs?: number | null
  /** The data template: data to display, never to interpret. */
  data?: unknown
}

/** "0 9 * * * (Europe/Paris)" / "every 900,000 ms"; `null` when unknown. */
export function describeSchedulerTiming(preview: SchedulerApprovalPreview): string | null {
  const pattern =
    preview.schedule?.kind === "pattern" ? preview.schedule.pattern : (preview.pattern ?? undefined)
  const tz = preview.schedule?.kind === "pattern" ? preview.schedule.tz : preview.tz
  const every =
    preview.schedule?.kind === "every" ? preview.schedule.everyMs : (preview.everyMs ?? undefined)
  if (pattern) return tz ? `${pattern} (${tz})` : pattern
  if (typeof every === "number") return `every ${every.toLocaleString()} ms`
  return null
}

export type TargetPreview =
  | JobPreview
  | BulkPreview
  | NewJobPreview
  | MaintenanceApprovalPreview
  | SchedulerApprovalPreview
  | { kind: "raw"; value: unknown }
  | null

/** Verb + object per tool. `destructive` styles Approve as dangerous: data is lost. */
const TOOL_PHRASES: Record<string, { verb: string; object: string; destructive?: boolean }> = {
  retry_job: { verb: "retry", object: "a job" },
  remove_job: { verb: "remove", object: "a job", destructive: true },
  promote_job: { verb: "promote", object: "a delayed job" },
  bulk_retry_jobs: { verb: "retry", object: "jobs" },
  bulk_remove_jobs: { verb: "remove", object: "jobs", destructive: true },
  bulk_promote_jobs: { verb: "promote", object: "delayed jobs" },
  pause_queue: { verb: "pause", object: "a queue" },
  resume_queue: { verb: "resume", object: "a queue" },
  pause_scheduler: { verb: "pause", object: "a scheduler" },
  resume_scheduler: { verb: "resume", object: "a scheduler" },
  add_job: { verb: "add", object: "a job" },
  drain_queue: { verb: "drain", object: "a queue", destructive: true },
  clean_queue: { verb: "clean", object: "a queue", destructive: true },
  upsert_scheduler: { verb: "create or change", object: "a scheduler" },
  remove_scheduler: { verb: "remove", object: "a scheduler", destructive: true },
}

export function parsePreview(json: string | null): TargetPreview {
  if (!json) return null
  let value: unknown
  try {
    value = JSON.parse(json)
  } catch {
    return null
  }
  if (value && typeof value === "object" && "kind" in value) {
    const kind = (value as { kind: unknown }).kind
    if (kind === "job") return value as JobPreview
    if (kind === "newJob") return value as NewJobPreview
    if (kind === "schedulerChange" || kind === "schedulerRemoval") {
      return value as SchedulerApprovalPreview
    }
    if (kind === "maintenance" && typeof (value as MaintenanceApprovalPreview).count === "number") {
      return value as MaintenanceApprovalPreview
    }
    if (kind === "bulk" && typeof (value as BulkPreview).count === "number") {
      return value as BulkPreview
    }
  }
  return { kind: "raw", value }
}

/**
 * "every waiting, paused and delayed job": what a maintenance call targets,
 * in the states the connection's backend describes.
 */
export function describeMaintenanceScope(
  preview: MaintenanceApprovalPreview,
  states: ReadonlyArray<JobStateInfo>,
): string {
  const op = preview.operation
  if (op?.kind === "drain") {
    return `every ${stateList(notStartedStates(states, op.includeDelayed !== false), states)} job`
  }
  if (op?.kind === "clean") {
    const age = op.olderThanMs ?? 0
    const state = op.state ? stateLabel(op.state, states).toLowerCase() : "finished"
    if (age === 0) return `every ${state} job`
    const hours = age / 3_600_000
    const span =
      hours >= 24 && Number.isInteger(hours / 24)
        ? `${hours / 24} day${hours / 24 === 1 ? "" : "s"}`
        : hours >= 1 && Number.isInteger(hours)
          ? `${hours} hour${hours === 1 ? "" : "s"}`
          : `${age.toLocaleString()} ms`
    return `${state} jobs that finished more than ${span} ago`
  }
  if (op?.kind === "retryAllFailed") return "every failed job"
  return "jobs"
}

export function isDestructive(tool: string): boolean {
  return TOOL_PHRASES[tool]?.destructive === true
}

export function clientLabel(approval: Pick<PendingApproval, "clientHint">): string {
  // `clientHint` is the raw `clientInfo.name` from the
  // MCP handshake (`local-agent-mode-ranch`); a title is a sentence, so it
  // gets the readable name. The raw identifier is shown beside it in the
  // modal, never replaced by the label.
  return agentLabel(approval.clientHint) ?? "An agent"
}

/** "Claude Desktop wants to remove 12 jobs": the modal's title. */
export function describeRequest(approval: PendingApproval): string {
  const who = clientLabel(approval)
  const phrase = TOOL_PHRASES[approval.tool]
  if (!phrase) {
    return approval.kind === "read"
      ? `${who} wants to read with ${approval.tool}`
      : `${who} wants to run ${approval.tool}`
  }
  const preview = parsePreview(approval.targetPreviewJson)
  if (preview?.kind === "bulk") {
    const noun = preview.count === 1 ? "job" : "jobs"
    return `${who} wants to ${phrase.verb} ${preview.count} ${noun}`
  }
  return `${who} wants to ${phrase.verb} ${phrase.object}`
}

/** Seconds left before the agent gives up (never negative). */
export function secondsLeft(approval: PendingApproval, nowMs: number): number {
  const createdMs = Date.parse(approval.createdAt)
  if (Number.isNaN(createdMs)) return approval.timeoutSeconds
  const left = approval.timeoutSeconds - Math.floor((nowMs - createdMs) / 1000)
  return Math.max(0, left)
}
