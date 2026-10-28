import { useEffect, useMemo } from "react"

import { useCapabilities } from "@/capabilities"
import { ProgressBar } from "@/components/ui/progress-bar"
import { useQueueMetrics } from "@/data/metrics/queries"
import { formatDurationMs } from "@/domain/format"
import type { Job, QueueId } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { jobDurationMs } from "./jobDuration"
import { parseJobProgress } from "./parseJobProgress"
import { progressKey, STALL_MS, stallOf, useProgressWatchStore } from "./progressWatch"

/**
 * The cells of the Active tab that need more than the
 * job: its progress (and whether it stopped moving), its running time
 * against the usual time of its type, its worker.
 */

/** Median running time per job name over the metrics window. */
export function useUsualDurations(connectionId: string, queueId: QueueId): Map<string, number> {
  const metered = useCapabilities(connectionId).can("metrics.queue")
  const { data } = useQueueMetrics(connectionId, metered ? queueId : undefined)
  return useMemo(
    () => new Map((data?.usualDurations ?? []).map((u) => [u.name, u.medianMs] as const)),
    [data],
  )
}

/** Remember when a running job's progress last changed; report a stall. */
export function useProgressStall(connectionId: string, job: Job, now: number) {
  const key = progressKey(connectionId, job.jobRef.queueId, job.jobRef.id)
  const observe = useProgressWatchStore((s) => s.observe)
  const watch = useProgressWatchStore((s) => s.seen.get(key))
  useEffect(() => {
    observe(key, job.progress, Date.now())
  }, [observe, key, job.progress])
  return stallOf(watch, job.lockTtlMs, now)
}

export function ActiveProgressCell({
  job,
  connectionId,
  now,
}: {
  job: Job
  connectionId: string
  now: number
}) {
  const parsed = parseJobProgress(job.progress)
  const { stalled } = useProgressStall(connectionId, job, now)
  return (
    <span
      className="flex min-w-0 flex-1 flex-col gap-0.5"
      data-testid="job-progress-cell"
      data-stalled={stalled ? "true" : undefined}
    >
      {parsed?.percent != null ? (
        <ProgressBar
          value={parsed.percent}
          label={parsed.label}
          ariaLabel="Job progress"
          tone={stalled ? "warning" : "active"}
        />
      ) : parsed ? (
        <span className="font-tabular text-xs break-words text-(--text)">{parsed.label}</span>
      ) : (
        <span className="font-tabular text-xs text-(--text2)">-</span>
      )}
      {stalled ? (
        <span className="text-2xs text-(--state-delayed-text)" data-testid="job-progress-stalled">
          no progress for {Math.round(STALL_MS / 60_000)} min
        </span>
      ) : parsed?.note ? (
        <span className="text-2xs break-words text-(--text2)" data-testid="job-progress-note">
          {parsed.note}
        </span>
      ) : null}
    </span>
  )
}

export function ActiveRunningCell({
  job,
  connectionId,
  now,
}: {
  job: Job
  connectionId: string
  now: number
}) {
  const usual = useUsualDurations(connectionId, job.jobRef.queueId).get(job.name)
  const ms = jobDurationMs(job, now)
  const slow = ms !== null && usual !== undefined && ms > usual
  return (
    <span className="flex flex-col items-end gap-0.5" data-testid="job-running-cell">
      <span
        className={cn(
          "font-tabular text-xs",
          slow ? "text-(--state-delayed-text)" : "text-(--text2)",
        )}
        data-slow={slow ? "true" : undefined}
        data-testid="job-running-time"
      >
        {ms === null ? "-" : formatDurationMs(ms)}
      </span>
      {usual !== undefined ? (
        <span className="text-2xs whitespace-nowrap text-(--text2)" data-testid="job-running-usual">
          usual {formatDurationMs(usual)}
        </span>
      ) : null}
    </span>
  )
}
