import type { Queue, QueueCounters } from "@/lib/tauri/bindings"

export type TotalState =
  | "waiting"
  | "active"
  | "completed"
  | "failed"
  | "delayed"
  | "prioritized"
  | "paused"

export const TOTAL_STATES: ReadonlyArray<{
  key: TotalState
  label: string
  /** The state's color (the same tokens as the chips and the charts). */
  dotClass: string
  /** Hidden while it holds nothing: most queues never use it. */
  onlyWhenUsed?: boolean
}> = [
  { key: "waiting", label: "Waiting", dotClass: "bg-[var(--state-waiting)]" },
  { key: "active", label: "Active", dotClass: "bg-[var(--state-active)]" },
  { key: "completed", label: "Completed", dotClass: "bg-[var(--state-completed)]" },
  { key: "failed", label: "Failed", dotClass: "bg-[var(--state-failed)]" },
  { key: "delayed", label: "Delayed", dotClass: "bg-[var(--state-delayed)]" },
  {
    key: "prioritized",
    label: "Prioritized",
    dotClass: "bg-[var(--state-waiting)]",
    onlyWhenUsed: true,
  },
  { key: "paused", label: "Paused", dotClass: "bg-amber-500", onlyWhenUsed: true },
]

export type StateTotals = Record<TotalState, number>

const ZERO: StateTotals = {
  waiting: 0,
  active: 0,
  completed: 0,
  failed: 0,
  delayed: 0,
  prioritized: 0,
  paused: 0,
}

const pick = (c: QueueCounters | null | undefined, key: TotalState): number =>
  Number((c as unknown as Record<string, number> | null | undefined)?.[key] ?? 0)

/** Jobs per state, summed over the queues (one queue = its own counters). */
export function sumCounters(queues: ReadonlyArray<Pick<Queue, "counters">>): StateTotals {
  const totals = { ...ZERO }
  for (const q of queues) {
    for (const { key } of TOTAL_STATES) totals[key] += pick(q.counters, key)
  }
  return totals
}
