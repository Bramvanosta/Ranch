import { useQueryClient } from "@tanstack/react-query"
import { useEffect, useState } from "react"

import { formatAge } from "@/features/jobs/formatAge"
import { cn } from "@/lib/utils"

export type FreshnessState =
  | { kind: "loading" }
  | { kind: "live"; ageMs: number }
  | { kind: "stale"; ageMs: number }
  | { kind: "failing"; ageMs: number; reason: string }
  | { kind: "down"; reason: string }

/** One query's timestamps, as the cache holds them. */
export interface QueryTimes {
  dataUpdatedAt: number
  errorUpdatedAt: number
  error: unknown
}

/**
 * How fresh a screen's data is, from its queries: the
 * oldest data on screen sets the age; data whose last refresh failed wins. "Stale" past
 * twice the refresh interval (a slow link, or polling stopped).
 */
export function freshnessOf(
  queries: QueryTimes[],
  intervalMs: number | false,
  now: number,
): FreshnessState {
  if (queries.length === 0) return { kind: "loading" }
  const loaded = queries.filter((q) => q.dataUpdatedAt > 0)
  if (loaded.length === 0) {
    const down = queries.find((q) => q.errorUpdatedAt > 0)
    return down ? { kind: "down", reason: errorText(down.error) } : { kind: "loading" }
  }
  // What is on screen and could not be refreshed. A query that never read
  // anything (an offline connection) shows nothing stale: its own row says
  // it is down.
  const failing = loaded.find((q) => q.errorUpdatedAt > q.dataUpdatedAt)
  const ageMs = Math.max(0, now - Math.min(...loaded.map((q) => q.dataUpdatedAt)))
  if (failing) return { kind: "failing", ageMs, reason: errorText(failing.error) }
  if (intervalMs !== false && ageMs > intervalMs * 2 + 1_000) return { kind: "stale", ageMs }
  return { kind: "live", ageMs }
}

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  return typeof error === "string" ? error : "Unknown error"
}

/**
 * The freshness chip: "Live", amber "Updated 12s ago" when the
 * data is late, red "Last update 40s ago · retrying" when the last refresh
 * failed. Reads every query whose key starts with `queryKey`; ticks every
 * second so the age moves.
 */
export function Freshness({
  queryKey,
  intervalMs,
  className,
}: {
  queryKey: readonly unknown[]
  /** The screen's refresh interval; `false` when auto-refresh is off. */
  intervalMs: number | false
  className?: string
}) {
  const queryClient = useQueryClient()
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1_000)
    return () => window.clearInterval(id)
  }, [])
  const queries = queryClient
    .getQueryCache()
    .findAll({ queryKey, type: "active" })
    .map((q) => ({
      dataUpdatedAt: q.state.dataUpdatedAt,
      errorUpdatedAt: q.state.errorUpdatedAt,
      error: q.state.error,
    }))
  const state = freshnessOf(queries, intervalMs, now)

  const { label, tone, title } = present(state, intervalMs)
  return (
    <span
      className={cn("inline-flex shrink-0 items-center gap-1.5 text-2xs", tone, className)}
      data-testid="freshness"
      data-state={state.kind}
      title={title}
      role="status"
      aria-live="polite"
    >
      <span
        aria-hidden="true"
        className={cn(
          "size-1.5 rounded-full bg-current",
          state.kind === "live" && intervalMs !== false && "animate-pulse",
        )}
      />
      {label}
    </span>
  )
}

function present(
  state: FreshnessState,
  intervalMs: number | false,
): { label: string; tone: string; title?: string } {
  switch (state.kind) {
    case "loading":
      return { label: "Loading", tone: "text-(--text2)" }
    case "live":
      return intervalMs === false
        ? { label: `Updated ${ago(state.ageMs)}`, tone: "text-(--text2)" }
        : {
            label: state.ageMs >= 2_000 ? `Live · ${formatAge(state.ageMs)}` : "Live",
            tone: "text-(--state-completed-text)",
            title: "Refreshed automatically",
          }
    case "stale":
      return {
        label: `Updated ${ago(state.ageMs)}`,
        tone: "text-(--state-delayed-text)",
        title: "Refreshes are taking longer than usual",
      }
    case "failing":
      return {
        label: `Last update ${ago(state.ageMs)} · retrying`,
        tone: "text-(--state-failed-text)",
        title: state.reason,
      }
    case "down":
      return {
        label: "Cannot refresh · retrying",
        tone: "text-(--state-failed-text)",
        title: state.reason,
      }
  }
}

function ago(ms: number): string {
  const age = formatAge(ms)
  return age === "now" ? "just now" : `${age} ago`
}
