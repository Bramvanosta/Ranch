import { useQuery } from "@tanstack/react-query"
import { useCallback, useEffect, useMemo, useReducer, useRef } from "react"
import { toast } from "sonner"

import { displayChord } from "@/commands/chords"
import { useCommandHandler } from "@/commands/handlers"
import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { Kbd } from "@/components/data/Kbd"
import { Button } from "@/components/ui/button"
import { withOneShotUnlock } from "@/features/safety/oneShotUnlock"
import { useConnectionLockState } from "@/features/safety/useConnectionLockState"
import { useKeyboardShortcut } from "@/hooks/useKeyboardShortcut"
import {
  type BulkActionResult,
  type Connection,
  commands,
  type QueueRef,
} from "@/lib/tauri/bindings"
import { useBulkProgressStore } from "@/stores/useBulkProgressStore"
import { type JobSelectionEntry, useJobSelectionStore } from "@/stores/useJobSelectionStore"

import { useBulkPromote, useBulkRemove, useBulkRetry } from "./useBulkAction"
import {
  type CrossQueueBulkResult,
  useBulkPromoteCrossQueue,
  useBulkRemoveCrossQueue,
  useBulkRetryCrossQueue,
} from "./useBulkActionCrossQueue"

/**
 * `BulkActionBar`: act on a selection of jobs, within one queue or across several.
 *
 * State machine (typed union):
 *   - `selecting`     : "{N} selected across {K} queues" + 3 action buttons + Cancel
 *   - `preview(act)`  : confirmation banner + Enter/Escape hint
 *   - `executing(...)`: progress bar + per-queue breakdown
 *   - `complete(...)` : structured summary, auto-dismiss after 6s
 *
 * Transitions:
 *   - selecting → preview(act)             : click action OR ⇧⌘R / ⌘⌫ / ⌥⌘P
 *   - preview → executing(act)             : Enter
 *   - preview → selecting                  : Escape
 *   - executing → complete(result)         : mutation resolves
 *   - executing → complete(synth-locked)   : mutation rejects with kind=connectionLocked
 *   - complete → selecting (selection)     : 6s timer OR next gesture
 *   - any → selecting (clear)              : Cancel button
 *
 * cross-queue (K>1) selections are now actionable. The
 * `fire(kind)` dispatcher branches:
 *   - K === 1 → `useBulkRetry / Remove / Promote` (3.4 single-queue path).
 *   - K  > 1 → `useBulkRetryCrossQueue / ... ` (3.5 frontend orchestrator).
 *
 * Preview / Executing / Complete views render per-queue breakdowns
 * when K > 1; single-queue layout when K === 1.
 */

export type BulkActionKind = "retry" | "remove" | "promote"

/** Discriminated result wrapper so Complete view knows whether the
 *  result is single-queue (3.4 shape) or cross-queue (3.5 shape).
 *  Each mode carries its own typed result. */
type CompleteResult =
  | { mode: "single"; result: BulkActionResult }
  | { mode: "cross"; result: CrossQueueBulkResult }

type BarState =
  | { kind: "selecting" }
  | { kind: "preview"; action: BulkActionKind }
  | { kind: "executing"; action: BulkActionKind }
  | { kind: "complete"; action: BulkActionKind; result: CompleteResult }

type BarAction =
  | { type: "preview"; action: BulkActionKind }
  | { type: "cancel" }
  | { type: "execute"; action: BulkActionKind }
  | { type: "complete"; action: BulkActionKind; result: CompleteResult }
  | { type: "reset" }

function reducer(state: BarState, action: BarAction): BarState {
  switch (action.type) {
    case "preview":
      if (state.kind !== "selecting") return state
      return { kind: "preview", action: action.action }
    case "cancel":
      return { kind: "selecting" }
    case "execute":
      if (state.kind !== "preview") return state
      return { kind: "executing", action: action.action }
    case "complete":
      return { kind: "complete", action: action.action, result: action.result }
    case "reset":
      return { kind: "selecting" }
  }
}

export interface BulkActionBarProps {
  connectionId: string
  queueRef: QueueRef
}

export function BulkActionBar({ connectionId, queueRef }: BulkActionBarProps) {
  const selection = useJobSelectionStore((s) => s.selection)
  const distinctQueues = useJobSelectionStore((s) => s.distinctQueues)
  const groupByQueue = useJobSelectionStore((s) => s.groupByQueue)
  const clear = useJobSelectionStore((s) => s.clear)
  const [state, dispatch] = useReducer(reducer, { kind: "selecting" } as BarState)

  // Single-queue (K=1) mutation hooks — 3.4 path.
  const retry = useBulkRetry()
  const remove = useBulkRemove()
  const promote = useBulkPromote()
  // Cross-queue (K>1) orchestrator hooks — 3.5 path.
  const retryCross = useBulkRetryCrossQueue()
  const removeCross = useBulkRemoveCrossQueue()
  const promoteCross = useBulkPromoteCrossQueue()

  const startProgress = useBulkProgressStore((s) => s.start)
  const finishProgress = useBulkProgressStore((s) => s.finish)
  // Subscribe to the per-queue maps so the Executing view re-renders
  // on every `tick()` from the global progress listener.
  const runningActions = useBulkProgressStore((s) => s.runningActions)
  const perQueueCompleted = useBulkProgressStore((s) => s.perQueueCompleted)
  const perQueueFailedSoFar = useBulkProgressStore((s) => s.perQueueFailedSoFar)
  const aggregateCompleted = useBulkProgressStore((s) => s.aggregateCompleted)
  const aggregateTotal = useBulkProgressStore((s) => s.aggregateTotal)
  const aggregateFailedSoFar = useBulkProgressStore((s) => s.aggregateFailedSoFar)

  // Pull the connection's env color for the accent stripe.
  const { data: connections } = useQuery({
    queryKey: ["connections"],
    queryFn: async () => {
      const r = await commands.listConnections()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    staleTime: 30_000,
  })
  const connection: Connection | undefined = connections?.find((c) => c.id === connectionId)
  const envColor = connection?.envColor ?? "env-local"
  const accentStyle = useMemo(
    () => ({
      borderLeftColor: `var(--${envColor})`,
      backgroundColor: `color-mix(in oklch, var(--${envColor}) 8%, var(--bg))`,
    }),
    [envColor],
  )

  const selectedCount = selection.size
  const queueNames = distinctQueues()
  const queueCount = queueNames.size
  const isCrossQueue = queueCount > 1

  // Auto-dismiss the complete summary after 6s of inactivity.
  const dismissTimer = useRef<number | null>(null)
  useEffect(() => {
    if (state.kind === "complete") {
      const handle = window.setTimeout(() => {
        dispatch({ type: "reset" })
      }, 6_000)
      dismissTimer.current = handle
      return () => {
        if (dismissTimer.current !== null) {
          window.clearTimeout(dismissTimer.current)
          dismissTimer.current = null
        }
      }
    }
    return undefined
  }, [state.kind])

  // `done` runs once the action has settled, so a one-shot
  // unlock can lock the connection again.
  const fire = useCallback(
    (kind: BulkActionKind, done?: () => void) => {
      dispatch({ type: "execute", action: kind })
      const groups = groupByQueue()

      if (!isCrossQueue) {
        // K=1 — 3.4 single-queue path.
        const firstEntry: JobSelectionEntry[] | undefined = groups.values().next().value
        const jobIds = (firstEntry ?? []).map((e) => e.jobRef.id)
        const queueRefSingle = firstEntry?.[0]?.jobRef.queue ?? queueRef
        const queueNameSingle = queueRefSingle.name

        startProgress(new Map([[queueNameSingle, { action: kind, total: jobIds.length, jobIds }]]))
        const input = { connectionId, queueRef: queueRefSingle, jobIds }
        const mutation = kind === "retry" ? retry : kind === "remove" ? remove : promote
        mutation.mutate(input, {
          onSuccess: (result) => {
            finishProgress()
            done?.()
            dispatch({
              type: "complete",
              action: kind,
              result: { mode: "single", result },
            })
          },
          onError: (err: unknown) => {
            finishProgress()
            done?.()
            const message = err instanceof Error ? err.message : String(err)
            // Whether locked or other error, surface as a synthesized
            // single-queue summary with one entry in `failed`. The
            // CompleteView renders the reason verbatim — for
            // `connectionLocked` the message already includes the
            // unlock keystroke (backend formatted it that way).
            dispatch({
              type: "complete",
              action: kind,
              result: {
                mode: "single",
                result: {
                  action: kind,
                  total: jobIds.length,
                  succeeded: 0,
                  failed: [{ jobId: "", reason: message }],
                  cancelled: false,
                },
              },
            })
          },
        })
        return
      }

      // K>1 — 3.5 cross-queue orchestrator path.
      const perQueueInit = new Map<
        string,
        { action: BulkActionKind; total: number; jobIds: string[] }
      >()
      for (const [name, entries] of groups.entries()) {
        perQueueInit.set(name, {
          action: kind,
          total: entries.length,
          jobIds: entries.map((e) => e.jobRef.id),
        })
      }
      startProgress(perQueueInit)
      const input = { connectionId, queueGroups: groups }
      const mutation =
        kind === "retry" ? retryCross : kind === "remove" ? removeCross : promoteCross
      mutation.mutate(input, {
        onSuccess: (result) => {
          finishProgress()
          done?.()
          dispatch({
            type: "complete",
            action: kind,
            result: { mode: "cross", result },
          })
        },
        // No onError — the cross-queue orchestrator collects
        // per-queue errors into `result.perQueue[i].error` and
        // resolves the mutation. A top-level rejection here would
        // only fire if `aggregateCrossQueueResults` itself threw
        // (which it doesn't); leaving onError unhandled is safe.
      })
    },
    [
      groupByQueue,
      isCrossQueue,
      queueRef,
      connectionId,
      retry,
      remove,
      promote,
      retryCross,
      removeCross,
      promoteCross,
      startProgress,
      finishProgress,
    ],
  )

  // On a protected connection the sheet asks for the
  // queue's name (the connection's when several queues are selected), and
  // the action runs under a one-shot, audited unlock.
  const { data: lockState } = useConnectionLockState(connectionId)
  const locked = lockState === "locked"
  const protectedName = locked
    ? isCrossQueue
      ? (connection?.name ?? connectionId)
      : (Array.from(queueNames)[0] ?? queueRef.name)
    : undefined
  const confirm = useCallback(
    (kind: BulkActionKind) => {
      if (!locked) {
        fire(kind)
        return
      }
      withOneShotUnlock(
        connectionId,
        () => new Promise<void>((resolve) => fire(kind, resolve)),
      ).catch((err: unknown) => {
        dispatch({ type: "cancel" })
        toast.error(err instanceof Error ? err.message : String(err))
      })
    },
    [locked, fire, connectionId],
  )

  const handleCancel = useCallback(() => {
    if (state.kind === "executing") {
      // Frontend-abandons-promise path: stop the UI here. The actor
      // continues server-side but we move on.
      finishProgress()
      const total = aggregateTotal()
      const completed = aggregateCompleted()
      const failedSoFar = aggregateFailedSoFar()
      dispatch({
        type: "complete",
        action: state.action,
        result: {
          mode: "single",
          result: {
            action: state.action,
            total,
            succeeded: Math.max(completed - failedSoFar, 0),
            failed: [],
            cancelled: true,
          },
        },
      })
    } else if (state.kind === "preview") {
      dispatch({ type: "cancel" })
    } else {
      clear()
      dispatch({ type: "reset" })
    }
  }, [state, finishProgress, aggregateTotal, aggregateCompleted, aggregateFailedSoFar, clear])

  // An action asked for from the list (`cmd-Backspace` on the focused row):
  // straight to the confirmation step.
  const requestedAction = useJobSelectionStore((s) => s.requestedAction)
  const requestAction = useJobSelectionStore((s) => s.requestAction)
  useEffect(() => {
    if (requestedAction && selectedCount > 0 && state.kind === "selecting") {
      dispatch({ type: "preview", action: requestedAction })
      requestAction(null)
    }
  }, [requestedAction, requestAction, selectedCount, state.kind])

  // Job › Retry / Promote / Remove act on the selection when
  // there is one (it outranks the focused row), and the menu says so.
  const selecting = state.kind === "selecting" && selectedCount > 0
  const jobs = selectedCount === 1 ? "1 Job" : `${selectedCount} Jobs`
  useCommandHandler("job.retry", () => dispatch({ type: "preview", action: "retry" }), {
    enabled: selecting,
    priority: 2,
    label: `Retry ${jobs}`,
  })
  useCommandHandler("job.promote", () => dispatch({ type: "preview", action: "promote" }), {
    enabled: selecting,
    priority: 2,
    label: `Promote ${jobs}`,
  })
  useCommandHandler("job.remove", () => dispatch({ type: "preview", action: "remove" }), {
    enabled: selecting,
    priority: 2,
    label: `Remove ${jobs}…`,
  })
  // Enter confirms what can be undone; a removal or a protected
  // connection needs its button (Cancel has the focus). A focused button
  // answers Enter itself.
  useKeyboardShortcut("enter", (e) => {
    if (state.kind !== "preview" || state.action === "remove" || locked) return
    if (e.target instanceof HTMLElement && e.target.closest("button, input")) return
    e.preventDefault()
    confirm(state.action)
  })
  useKeyboardShortcut("escape", (e) => {
    if (state.kind !== "selecting") {
      e.preventDefault()
      handleCancel()
    }
  })

  if (selectedCount === 0 && state.kind !== "complete") {
    return null
  }

  // Per-queue groupings for sub-views (K>1 cases).
  const groupsForViews = groupByQueue()

  return (
    <section
      aria-live="polite"
      aria-label="Bulk action bar"
      data-testid="bulk-action-bar"
      data-state={state.kind}
      className="flex w-full items-center gap-3 border-b border-l-4 px-4 py-2 text-sm"
      style={accentStyle}
    >
      {(state.kind === "selecting" || state.kind === "preview") && (
        <SelectingView
          selectedCount={selectedCount}
          queueCount={queueCount}
          onAction={(act) => dispatch({ type: "preview", action: act })}
          onCancel={handleCancel}
        />
      )}
      <PreviewSheet
        open={state.kind === "preview"}
        action={state.kind === "preview" ? state.action : "retry"}
        selectedCount={selectedCount}
        queueCount={queueCount}
        isCrossQueue={isCrossQueue}
        groups={groupsForViews}
        protectedName={protectedName}
        onExecute={() => state.kind === "preview" && confirm(state.action)}
        onCancel={handleCancel}
      />
      {state.kind === "executing" && (
        <ExecutingView
          action={state.action}
          isCrossQueue={isCrossQueue}
          totalAggregate={aggregateTotal()}
          completedAggregate={aggregateCompleted()}
          failedSoFarAggregate={aggregateFailedSoFar()}
          perQueueTotals={
            new Map(Array.from(runningActions.entries()).map(([k, v]) => [k, v.total]))
          }
          perQueueCompleted={perQueueCompleted}
          perQueueFailedSoFar={perQueueFailedSoFar}
          singleQueueName={queueRef.name}
          onCancel={handleCancel}
        />
      )}
      {state.kind === "complete" && <CompleteView result={state.result} onDismiss={handleCancel} />}
    </section>
  )
}

function SelectingView(props: {
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

function PreviewSheet(props: {
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
          {Array.from(props.groups.entries()).map(([queueName, entries]) => (
            <li key={queueName} className="flex justify-between gap-3">
              <span className="min-w-0 font-mono break-words">{queueName}</span>
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

function ExecutingView(props: {
  action: BulkActionKind
  isCrossQueue: boolean
  totalAggregate: number
  completedAggregate: number
  failedSoFarAggregate: number
  perQueueTotals: Map<string, number>
  perQueueCompleted: Map<string, number>
  perQueueFailedSoFar: Map<string, number>
  singleQueueName: string
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
              .map(([qn, qt]) => `${qn} ${props.perQueueCompleted.get(qn) ?? 0}/${qt}`)
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

function CompleteView(props: { result: CompleteResult; onDismiss: () => void }) {
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
              <li className="italic">…and {r.failed.length - 5} more</li>
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
    // Use the first per-queue error verbatim — they're all the same
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
            <li key={pq.queueName}>
              <span className="font-mono">{pq.queueName}</span>:{" "}
              {pq.error ? (
                <span className="text-(--state-failed-text)">blocked — {pq.error}</span>
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
