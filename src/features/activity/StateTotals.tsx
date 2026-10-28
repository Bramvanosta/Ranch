import type { ReactNode } from "react"
import type { JobStateInfo } from "@/domain/jobStates"
import { countOf } from "@/domain/stateCounts"
import { totalStates } from "@/domain/sumCounters"
import type { JobState, StateCounts } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

interface StateTotalsProps {
  totals: StateCounts
  /** The states the connection's backend keeps (`useJobStates`). */
  states: ReadonlyArray<JobStateInfo>
  /** Wraps a tile in whatever takes the user to those jobs; `null` = plain tile. */
  linkFor?: (state: JobState, tile: ReactNode) => ReactNode | null
  className?: string
}

/**
 * Jobs per state, right now: the live counters of a queue, or their sum over
 * a connection. *Completed* and *Failed* are what the queues **keep**
 * (`removeOnComplete` / `removeOnFail`), not all-time totals: the tiles say so.
 */
export function StateTotals({ totals, states, linkFor, className }: StateTotalsProps) {
  const shown = totalStates(states, totals)
  return (
    <section
      aria-label="Jobs by state"
      data-testid="state-totals"
      className={cn("@container", className)}
    >
      <ul className="grid grid-cols-[repeat(auto-fit,minmax(7.5rem,1fr))] gap-2">
        {shown.map((s) => {
          const n = countOf(totals, s.code)
          const kept = s.code === "completed" || s.code === "failed"
          const tile = (
            <div
              className="flex h-full flex-col gap-0.5 rounded-lg border border-(--line) bg-(--surf) px-3 py-2"
              title={
                kept
                  ? `${n.toLocaleString()} ${s.label.toLowerCase()} jobs kept by the queue (removeOn${s.code === "completed" ? "Complete" : "Fail"}), not an all-time total`
                  : `${n.toLocaleString()} ${s.label.toLowerCase()} jobs`
              }
            >
              <span className="flex items-center gap-1.5 text-xs text-(--text2)">
                <span aria-hidden="true" className={cn("size-2 rounded-full", s.dotClass)} />
                {s.label}
                {kept ? <span className="text-2xs text-(--text2)">kept</span> : null}
              </span>
              <span
                className="font-tabular text-xl font-semibold leading-tight"
                data-testid={`state-total-${s.code}`}
              >
                {n.toLocaleString()}
              </span>
            </div>
          )
          return (
            <li key={s.code} className="min-w-0">
              {linkFor?.(s.code, tile) ?? tile}
            </li>
          )
        })}
      </ul>
    </section>
  )
}
