import { useState } from "react"
import { toast } from "sonner"
import { useJobStates } from "@/capabilities"
import { ConfirmSheet, type CountLine } from "@/components/data/ConfirmSheet"
import { Label } from "@/components/ui/label"
import { ProgressBar } from "@/components/ui/progress-bar"
import { runQueueMaintenance, useInvalidateAfterMaintenance } from "@/data/queues/mutations"
import { useMaintenancePreview, useQueues } from "@/data/queues/queries"
import { withOneShotUnlock } from "@/data/safety/oneShotUnlock"
import { useConnectionLockState } from "@/data/safety/queries"
import { countOf } from "@/domain/stateCounts"
import type {
  CleanState,
  JobState,
  MaintenanceResult,
  QueueId,
  QueueMaintenance,
  StateCounts,
} from "@/lib/tauri/bindings"
import { useBulkProgressStore } from "@/stores/useBulkProgressStore"
import {
  CLEAN_STATES,
  describePreview,
  describeResult,
  type MaintenanceKind,
  maintenanceCopy,
  OLDER_THAN_OPTIONS,
} from "./maintenance"

/**
 * preview-then-confirm for a queue maintenance operation: the dialog states how many jobs the operation touches *before* it
 * can be confirmed, re-counts when the options change, and shows progress
 * while the actor works through the queue in batches.
 */
interface QueueMaintenanceDialogProps {
  /** `null` = closed. */
  kind: MaintenanceKind | null
  onClose: () => void
  connectionId: string
  queueId: QueueId
}

export function QueueMaintenanceDialog({
  kind,
  onClose,
  connectionId,
  queueId,
}: QueueMaintenanceDialogProps) {
  const invalidateAfterMaintenance = useInvalidateAfterMaintenance()
  // The live counters say what stays; the lock says whether to ask for the name.
  const { data: queues } = useQueues(connectionId)
  const queue = queues?.find((q) => q.id === queueId)
  const queueName = queue?.name ?? queueId
  const states = useJobStates(connectionId)
  const { data: lockState } = useConnectionLockState(connectionId)
  const locked = lockState === "locked"
  const [includeDelayed, setIncludeDelayed] = useState(true)
  const [cleanState, setCleanState] = useState<CleanState>("completed")
  const [olderThanMs, setOlderThanMs] = useState(0)
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const operation: QueueMaintenance | null =
    kind === "drain"
      ? { kind: "drain", includeDelayed }
      : kind === "clean"
        ? { kind: "clean", state: cleanState, olderThanMs }
        : kind === "retryAllFailed"
          ? { kind: "retryAllFailed" }
          : null

  const preview = useMaintenancePreview(connectionId, queueId, operation, !running)

  // The actor's `bulk:progress` events land in this store (App root bridge).
  const done = useBulkProgressStore((s) => s.perQueueCompleted.get(queueId) ?? 0)
  const tick = useBulkProgressStore((s) => s.tick)

  function close() {
    if (running) return
    setError(null)
    onClose()
  }

  async function run() {
    if (!operation) return
    setError(null)
    setRunning(true)
    tick(queueId, 0, 0)
    const send = () => runQueueMaintenance(connectionId, queueId, operation)
    // A protected connection runs a destructive operation under a one-shot unlock.
    const response = await (locked ? withOneShotUnlock(connectionId, send) : send()).catch(
      (e: unknown) =>
        ({
          status: "error",
          error: { message: e instanceof Error ? e.message : String(e) },
        }) as const,
    )
    setRunning(false)
    invalidateAfterMaintenance(connectionId)
    if (response.status === "error") {
      setError(response.error.message)
      return
    }
    report(response.data)
    onClose()
  }

  function report(result: MaintenanceResult) {
    const message = describeResult(result)
    if (result.cancelled) toast.warning(`${queueName}: ${message}`)
    else toast.success(`${queueName}: ${message}`)
  }

  if (!kind || !operation) return null
  const copy = maintenanceCopy(kind, queueName, states)
  const total = preview.data?.affected ?? 0
  const nothingToDo = preview.data !== undefined && total === 0
  const counts = sheetCounts(operation, preview.data?.affected, queue?.counts)
  const label = confirmLabel(operation, preview.data?.affected)

  return (
    <ConfirmSheet
      open
      onOpenChange={(open) => !open && close()}
      title={`${copy.title}?`}
      description={copy.description}
      danger={copy.destructive}
      removed={counts.removed}
      kept={counts.kept}
      // A protected connection asks for the queue's name, and runs
      // the action under a one-shot, audited unlock.
      protectedName={locked ? queueName : undefined}
      confirmLabel={running ? "Working" : label}
      onConfirm={() => void run()}
      busy={running}
      // Nothing can be confirmed before the count is on screen.
      disabled={!preview.data || nothingToDo || preview.isFetching}
      testId="queue-maintenance-dialog"
      confirmTestId="queue-maintenance-confirm"
    >
      {kind === "drain" ? (
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={includeDelayed}
            disabled={running}
            onChange={(e) => setIncludeDelayed(e.target.checked)}
            className="size-4 rounded border border-(--line)"
          />
          Also remove delayed jobs
        </label>
      ) : null}

      {kind === "clean" ? (
        <div className="grid grid-cols-2 gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="maintenance-state">Jobs</Label>
            <select
              id="maintenance-state"
              value={cleanState}
              disabled={running}
              onChange={(e) => setCleanState(e.target.value as CleanState)}
              className="h-control rounded-control border border-(--line) bg-(--field) px-2 text-sm"
            >
              {CLEAN_STATES.map((s) => (
                <option key={s.value} value={s.value}>
                  {s.label}
                </option>
              ))}
            </select>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="maintenance-age">Finished</Label>
            <select
              id="maintenance-age"
              value={olderThanMs}
              disabled={running}
              onChange={(e) => setOlderThanMs(Number(e.target.value))}
              className="h-control rounded-control border border-(--line) bg-(--field) px-2 text-sm"
            >
              {OLDER_THAN_OPTIONS.map((o) => (
                <option key={o.ms} value={o.ms}>
                  {o.label}
                </option>
              ))}
            </select>
          </div>
        </div>
      ) : null}

      <div
        className="rounded-card border border-(--line) bg-(--surf) p-3 text-sm"
        data-testid="queue-maintenance-preview"
        aria-live="polite"
      >
        {running ? (
          <div className="flex flex-col gap-2">
            <span>
              {Math.min(done, total).toLocaleString()} of {total.toLocaleString()} jobs
            </span>
            <ProgressBar value={done} max={total} ariaLabel="Progress" />
          </div>
        ) : preview.isError ? (
          <span className="text-(--state-failed-text)">
            Could not count the jobs:{" "}
            {preview.error instanceof Error ? preview.error.message : "unknown error"}
          </span>
        ) : preview.data ? (
          <span className="font-medium">{describePreview(operation, preview.data)}</span>
        ) : (
          <span className="text-(--text2)">Counting</span>
        )}
      </div>

      {error ? (
        <p
          role="alert"
          className="rounded-card border border-(--danger)/40 bg-(--danger)/10 p-2 text-xs text-(--danger)"
        >
          {error}
        </p>
      ) : null}
    </ConfirmSheet>
  )
}

/** "Drain 400 jobs": what the button does, with the count. */
function confirmLabel(op: QueueMaintenance, affected: number | undefined): string {
  const verb = op.kind === "drain" ? "Drain" : op.kind === "clean" ? "Clean" : "Retry"
  if (affected === undefined) return verb
  return `${verb} ${affected.toLocaleString()} ${affected === 1 ? "job" : "jobs"}`
}

/** What goes and what stays, from the preview and the queue's live counters. */
function sheetCounts(
  op: QueueMaintenance,
  affected: number | undefined,
  counts: StateCounts | undefined,
): { removed: CountLine[]; kept: CountLine[] } {
  if (affected === undefined) return { removed: [], kept: [] }
  const c = (state: JobState) => countOf(counts, state)
  if (op.kind === "retryAllFailed") {
    return {
      removed: [{ label: "Failed jobs moved back to waiting", count: affected }],
      kept: [{ label: "Every other job", count: undefined }],
    }
  }
  if (op.kind === "drain") {
    return {
      removed: [
        {
          label: op.includeDelayed ? "Waiting and delayed jobs" : "Waiting jobs",
          count: affected,
        },
      ],
      kept: [
        { label: "Active", count: c("active") },
        ...(op.includeDelayed ? [] : [{ label: "Delayed", count: c("delayed") }]),
        { label: "Failed", count: c("failed") },
        { label: "Completed", count: c("completed") },
      ],
    }
  }
  const stateCount = c(op.state)
  return {
    removed: [{ label: `${op.state[0]?.toUpperCase()}${op.state.slice(1)} jobs`, count: affected }],
    kept: [
      { label: `Other ${op.state} jobs`, count: Math.max(stateCount - affected, 0) },
      { label: "Jobs in every other state", count: undefined },
    ],
  }
}
