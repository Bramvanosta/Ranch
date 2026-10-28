import { displayChord } from "@/commands/chords"
import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { Kbd } from "@/components/data/Kbd"
import { Button } from "@/components/ui/button"
import type { BulkActionKind, CrossQueueBulkResult } from "@/data/jobs/bulkMutations"
import type { BulkActionResult } from "@/lib/tauri/bindings"
import type { JobSelectionEntry } from "@/stores/useJobSelectionStore"

import type { CompleteResult } from "./bulkActionState"

/* The views of the bulk action bar, one per state (`bulkActionState.ts`). */

export function SelectingView(props: {
  selectedCount: number
  queueCount: number
  onAction: (k: BulkActionKind) => void
  onCancel: () => void
}) {
  // K>1 no longer disables actions; cross-queue is
  // actionable via the frontend orchestrator.
  const button = (label: string, kind: BulkActionKind, kbdHint: string) => (
    <Button
      type="button"
      size="sm"
      variant="default"
      onClick={() => props.onAction(kind)}
      data-testid={`bulk-action-${kind}`}
      className="gap-1.5"
    >
      <span>{label}</span>
      <Kbd className="hidden sm:inline-flex">{kbdHint}</Kbd>
    </Button>
  )
  return (
    <>
      <span className="flex-1" data-testid="bulk-action-bar-count">
        <strong>{props.selectedCount}</strong> selected across <strong>{props.queueCount}</strong>{" "}
        {props.queueCount === 1 ? "queue" : "queues"}
      </span>
      {button("Retry", "retry", displayChord("cmd+shift+r"))}
      {button("Remove", "remove", displayChord("cmd+backspace"))}
      {button("Promote", "promote", displayChord("alt+cmd+p"))}
      <Button
        type="button"
        size="sm"
        variant="ghost"
        onClick={props.onCancel}
        data-testid="bulk-action-bar-cancel"
      >
        Cancel
      </Button>
    </>
  )
}

export function PreviewSheet(props: {
  open: boolean
  action: BulkActionKind
  selectedCount: number
  queueCount: number
  isCrossQueue: boolean
  groups: Map<string, JobSelectionEntry[]>
  protectedName?: string
  onExecute: () => void
  onCancel: () => void
}) {
  const verb = props.action === "retry" ? "Retry" : props.action === "remove" ? "Remove" : "Promote"
  const queueWord = props.queueCount === 1 ? "queue" : "queues"
  const irreversible = props.action === "remove"
  const jobWord = props.selectedCount === 1 ? "job" : "jobs"
  const removed = props.action === "remove"
  const kept =
    props.action === "remove"
      ? `Every other job of ${props.isCrossQueue ? "these queues" : "this queue"}`
      : "Their data, attempts and logs"

  return (
    <ConfirmSheet
      open={props.open}
      onOpenChange={(open) => {
        if (!open) props.onCancel()
      }}
      title={`${verb} ${props.selectedCount} ${jobWord}?`}
      description={
        <span data-testid="bulk-action-bar-preview">
          {verb} {props.selectedCount} {jobWord} across {props.queueCount} {queueWord}.
          {irreversible ? " This is irreversible." : " This will move them to waiting state."}
        </span>
      }
      danger={removed}
      removed={
        props.isCrossQueue ? [] : [{ label: `Selected ${jobWord}`, count: props.selectedCount }]
      }
      kept={[{ label: kept }]}
      protectedName={props.protectedName}
      confirmLabel={`${verb} ${props.selectedCount} ${jobWord}`}
      onConfirm={props.onExecute}
      testId="bulk-action-bar-sheet"
      confirmTestId="bulk-action-bar-execute"
      cancelTestId="bulk-action-bar-preview-cancel"
    >
      {props.isCrossQueue ? (
        <ul
          className="flex flex-col gap-1 rounded-card border border-(--line) bg-(--surf) p-3 text-sm"
          data-testid="bulk-action-bar-preview-breakdown"
        >
          {Array.from(props.groups.entries()).map(([queueId, entries]) => (
            <li key={queueId} className="flex justify-between gap-3">
              <span className="min-w-0 font-mono break-words">
                {entries[0]?.jobRef.queueName ?? queueId}
              </span>
              <span className="font-tabular">{entries.length.toLocaleString()}</span>
            </li>
          ))}
        </ul>
      ) : null}
      {irreversible ? null : (
        <p className="text-xs text-(--text2)">
          <Kbd>Enter</Kbd> to confirm, <Kbd>Esc</Kbd> to cancel.
        </p>
      )}
    </ConfirmSheet>
  )
}

export function ExecutingView(props: {
  action: BulkActionKind
  isCrossQueue: boolean
  totalAggregate: number
  completedAggregate: number
  failedSoFarAggregate: number
  perQueueTotals: Map<string, number>
  perQueueCompleted: Map<string, number>
  perQueueFailedSoFar: Map<string, number>
  singleQueueName: string
  /** The queues' names, by id. */
  queueNames: Map<string, string>
  onCancel: () => void
}) {
  const pct =
    props.totalAggregate > 0
      ? Math.min(100, Math.round((props.completedAggregate / props.totalAggregate) * 100))
      : 0
  const verb =
    props.action === "retry" ? "Retrying" : props.action === "remove" ? "Removing" : "Promoting"
  return (
    <>
      <span className="flex-1" data-testid="bulk-action-bar-executing">
        {verb} <strong>{props.completedAggregate}</strong> of{" "}
        <strong>{props.totalAggregate}</strong>
        {props.isCrossQueue ? (
          <>
            {" "}
            across <strong>{props.perQueueTotals.size}</strong> queues
          </>
        ) : (
          <>
            {" "}
            in queue <span className="font-mono">{props.singleQueueName}</span>
          </>
        )}
        {props.failedSoFarAggregate > 0 ? (
          <span className="ml-2 text-(--state-failed-text)">
            ({props.failedSoFarAggregate} failed)
          </span>
        ) : null}
        {props.isCrossQueue ? (
          <small className="ml-2 text-(--text2)" data-testid="bulk-action-bar-executing-breakdown">
            {Array.from(props.perQueueTotals.entries())
              .map(
                ([id, qt]) =>
                  `${props.queueNames.get(id) ?? id} ${props.perQueueCompleted.get(id) ?? 0}/${qt}`,
              )
              .join(" · ")}
          </small>
        ) : null}
      </span>
      <div
        className="h-2 w-32 overflow-hidden rounded-full bg-(--surf)"
        data-testid="bulk-progress-bar"
      >
        <div
          className="h-full bg-(--accent) transition-[width] duration-150"
          style={{ width: `${pct}%` }}
        />
      </div>
      <Button
        type="button"
        size="sm"
        variant="ghost"
        onClick={props.onCancel}
        data-testid="bulk-action-bar-cancel"
      >
        Cancel
      </Button>
    </>
  )
}

export function CompleteView(props: { result: CompleteResult; onDismiss: () => void }) {
  if (props.result.mode === "single") {
    return <CompleteSingleView result={props.result.result} onDismiss={props.onDismiss} />
  }
  return <CompleteCrossView result={props.result.result} onDismiss={props.onDismiss} />
}

function CompleteSingleView(props: { result: BulkActionResult; onDismiss: () => void }) {
  const r = props.result
  const verb = r.action === "retry" ? "Retried" : r.action === "remove" ? "Removed" : "Promoted"
  let summary: string
  if (r.cancelled) {
    summary = `Cancelled. ${verb} ${r.succeeded} of ${r.total} before stopping.`
  } else if (r.failed.length === 0) {
    summary = `${verb} ${r.succeeded} ${r.succeeded === 1 ? "job" : "jobs"} successfully.`
  } else {
    summary = `${verb} ${r.succeeded} of ${r.total}. ${r.failed.length} failed (see details).`
  }
  return (
    <>
      <span className="flex-1" data-testid="bulk-action-bar-complete">
        {summary}
        {r.failed.length > 0 ? (
          <ul className="mt-1 list-inside list-disc text-2xs text-(--text2)">
            {r.failed.slice(0, 5).map((f, idx) => (
              <li
                // biome-ignore lint/suspicious/noArrayIndexKey: failure list is stable for the run
                key={idx}
              >
                {f.jobId ? <span className="font-mono">#{f.jobId}</span> : null}
                {f.jobId ? " · " : ""}
                {f.reason}
              </li>
            ))}
            {r.failed.length > 5 ? (
              <li className="italic">and {r.failed.length - 5} more</li>
            ) : null}
          </ul>
        ) : null}
      </span>
      <Button
        type="button"
        size="sm"
        variant="ghost"
        onClick={props.onDismiss}
        data-testid="bulk-action-bar-dismiss"
      >
        Dismiss
      </Button>
    </>
  )
}

function CompleteCrossView(props: { result: CrossQueueBulkResult; onDismiss: () => void }) {
  const r = props.result
  const verb = r.action === "retry" ? "Retried" : r.action === "remove" ? "Removed" : "Promoted"

  // When EVERY per-queue outcome carries a
  // connectionLocked-style error, consolidate the message to a
  // single human line. The backend's locked error message always
  // includes "is locked" so we test that pattern.
  const allLocked =
    r.perQueue.length > 0 && r.perQueue.every((pq) => pq.error?.includes("is locked"))

  if (allLocked) {
    // Use the first per-queue error verbatim; they're all the same
    // connection-level lock message (same unlock hint).
    const firstError = r.perQueue[0]?.error ?? ""
    return (
      <>
        <span className="flex-1" data-testid="bulk-action-bar-complete-locked">
          All {r.total} jobs blocked: {firstError}
        </span>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          onClick={props.onDismiss}
          data-testid="bulk-action-bar-dismiss"
        >
          Dismiss
        </Button>
      </>
    )
  }

  const headline = r.cancelled
    ? `Cancelled. ${verb} ${r.succeeded} of ${r.total} across ${r.perQueue.length} queues before stopping.`
    : r.failed.length === 0 && r.perQueue.every((pq) => !pq.error)
      ? `${verb} ${r.succeeded} ${r.succeeded === 1 ? "job" : "jobs"} successfully across ${r.perQueue.length} queues.`
      : `${verb} ${r.succeeded} of ${r.total} across ${r.perQueue.length} queues.`

  return (
    <>
      <span className="flex-1" data-testid="bulk-action-bar-complete">
        {headline}
        <ul
          className="mt-1 list-inside list-disc text-2xs text-(--text2)"
          data-testid="bulk-action-bar-complete-breakdown"
        >
          {r.perQueue.map((pq) => (
            <li key={pq.queueId}>
              <span className="font-mono">{pq.queueName}</span>:{" "}
              {pq.error ? (
                <span className="text-(--state-failed-text)">blocked: {pq.error}</span>
              ) : (
                <>
                  {pq.succeeded}/{pq.total}
                  {pq.failed.length > 0 ? ` (${pq.failed.length} failed)` : ""}
                </>
              )}
            </li>
          ))}
        </ul>
      </span>
      <Button
        type="button"
        size="sm"
        variant="ghost"
        onClick={props.onDismiss}
        data-testid="bulk-action-bar-dismiss"
      >
        Dismiss
      </Button>
    </>
  )
}
