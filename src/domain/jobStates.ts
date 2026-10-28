import type { JobState, JobStateInfo, StateTone } from "@/lib/tauri/bindings"

/**
 * How screens show a job state. A state crosses the wire as its code; the
 * connection's backend describes the states it keeps (code, label, tone),
 * and a screen reads them with `useJobStates` (`src/capabilities/`). These
 * helpers take that description and own no list of states.
 */

export type { JobStateInfo, StateTone }

/** `code` as `states` describe it; a code they do not describe reads as itself, neutral. */
export function describeState(code: JobState, states: ReadonlyArray<JobStateInfo>): JobStateInfo {
  return (
    states.find((s) => s.code === code) ?? { code, label: code, tone: "neutral", common: false }
  )
}

export function stateLabel(code: JobState, states: ReadonlyArray<JobStateInfo>): string {
  return describeState(code, states).label
}

/**
 * The lifecycle states a queue always shows, when its backend keeps them,
 * in the order work moves through them. Every other state it keeps (its
 * own, and jobs held by a pause) shows only while it holds jobs.
 */
export const HEADLINE_STATES: ReadonlyArray<JobState> = [
  "waiting",
  "active",
  "delayed",
  "failed",
  "completed",
]

export function isHeadline(code: JobState): boolean {
  return HEADLINE_STATES.includes(code)
}

/** A dot of the state's colour, as the totals and the charts draw it. */
export const TONE_DOT: Record<StateTone, string> = {
  neutral: "bg-(--text2)",
  pending: "bg-(--state-waiting)",
  running: "bg-(--state-active)",
  success: "bg-(--state-completed)",
  warning: "bg-(--state-delayed)",
  danger: "bg-(--state-failed)",
}

/**
 * The states whose jobs wait for a worker: `waiting`, and every other state
 * `states` describes as pending (a backend's own way of waiting, such as an
 * order by priority). Jobs held by a pause are not among them.
 */
export function waitingStates(states: ReadonlyArray<JobStateInfo>): JobState[] {
  const own = states
    .filter((s) => s.tone === "pending" && s.code !== "waiting" && s.code !== "paused")
    .map((s) => s.code)
  return ["waiting", ...own]
}

/**
 * The states of the jobs that have not started, as a drain removes them:
 * waiting, held by a pause, the backend's other pending states, and
 * delayed when included.
 */
export function notStartedStates(
  states: ReadonlyArray<JobStateInfo>,
  withDelayed: boolean,
): JobState[] {
  const [waiting, ...own] = waitingStates(states)
  return [waiting, "paused", ...own, ...(withDelayed ? ["delayed"] : [])]
}

/** "waiting, paused and delayed": the states' labels, in lower case, as a sentence lists them. */
export function stateList(codes: ReadonlyArray<JobState>, states: ReadonlyArray<JobStateInfo>) {
  const words = codes.map((code) => stateLabel(code, states).toLowerCase())
  return words.length < 2
    ? words.join("")
    : `${words.slice(0, -1).join(", ")} and ${words[words.length - 1]}`
}
