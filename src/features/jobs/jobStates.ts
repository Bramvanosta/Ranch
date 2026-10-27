import type { JobState, QueueCounters } from "@/lib/tauri/bindings"

/**
 * The states a queue's job list shows, in tab order: the five
 * BullMQ headline states, then Prioritized and Paused, which only appear
 * when they hold jobs (or are the one open) — most queues never use them,
 * but a job added with a priority, or to a paused queue, must be findable.
 */
export type ChipState =
  | "waiting"
  | "active"
  | "delayed"
  | "failed"
  | "completed"
  | "prioritized"
  | "paused"

export const STATE_TABS: ReadonlyArray<{ key: ChipState; label: string; onlyWhenUsed?: boolean }> =
  [
    { key: "waiting", label: "Waiting" },
    { key: "active", label: "Active" },
    { key: "delayed", label: "Delayed" },
    { key: "failed", label: "Failed" },
    { key: "completed", label: "Completed" },
    { key: "prioritized", label: "Prioritized", onlyWhenUsed: true },
    { key: "paused", label: "Paused", onlyWhenUsed: true },
  ]

export const CHIP_STATES: ReadonlyArray<ChipState> = STATE_TABS.map((s) => s.key)

/** The state tabs to show for these counters: every headline state, the rare ones when used. */
export function visibleStateTabs(counters: QueueCounters | undefined, open: JobState | undefined) {
  return STATE_TABS.filter(
    (s) => !s.onlyWhenUsed || s.key === open || Number(counters?.[s.key] ?? 0) > 0,
  )
}
