import type { ReactNode } from "react"

import { cn } from "@/lib/utils"

import { TOTAL_STATES, type TotalState, type StateTotals as Totals } from "./sumCounters"

interface StateTotalsProps {
  totals: Totals
  /** Wraps a tile in whatever takes the user to those jobs; `null` = plain tile. */
  linkFor?: (state: TotalState, tile: ReactNode) => ReactNode | null
  className?: string
}

/**
 * Jobs per state, right now — the live counters of a queue, or their sum over
 * a connection. *Completed* and *Failed* are what the queues **keep**
 * (`removeOnComplete` / `removeOnFail`), not all-time totals: the tiles say so.
 */
export function StateTotals({ totals, linkFor, className }: StateTotalsProps) {
  const shown = TOTAL_STATES.filter((s) => !s.onlyWhenUsed || totals[s.key] > 0)
  return (
    <section
      aria-label="Jobs by state"
      data-testid="state-totals"
      className={cn("@container", className)}
    >
      <ul className="grid grid-cols-[repeat(auto-fit,minmax(7.5rem,1fr))] gap-2">
        {shown.map((s) => {
          const kept = s.key === "completed" || s.key === "failed"
          const tile = (
            <div
              className="flex h-full flex-col gap-0.5 rounded-lg border border-(--line) bg-(--surf) px-3 py-2"
              title={
                kept
                  ? `${totals[s.key].toLocaleString()} ${s.label.toLowerCase()} jobs kept by the queue (removeOn${s.key === "completed" ? "Complete" : "Fail"}) — not an all-time total`
                  : `${totals[s.key].toLocaleString()} ${s.label.toLowerCase()} jobs`
              }
            >
              <span className="flex items-center gap-1.5 text-xs text-(--text2)">
                <span aria-hidden="true" className={cn("size-2 rounded-full", s.dotClass)} />
                {s.label}
                {kept ? <span className="text-2xs text-(--text2)">kept</span> : null}
              </span>
              <span
                className="font-tabular text-xl font-semibold leading-tight"
                data-testid={`state-total-${s.key}`}
              >
                {totals[s.key].toLocaleString()}
              </span>
            </div>
          )
          return (
            <li key={s.key} className="min-w-0">
              {linkFor?.(s.key, tile) ?? tile}
            </li>
          )
        })}
      </ul>
    </section>
  )
}
