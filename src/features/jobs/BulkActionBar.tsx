import { useCallback, useEffect, useReducer, useRef } from "react"
import { toast } from "sonner"

import { useCapabilities } from "@/capabilities"
import { useCommandHandler } from "@/commands/handlers"
import { useConnections } from "@/data/connections/queries"
import {
  type BulkActionKind,
  useBulkPromote,
  useBulkPromoteCrossQueue,
  useBulkRemove,
  useBulkRemoveCrossQueue,
  useBulkRetry,
  useBulkRetryCrossQueue,
} from "@/data/jobs/bulkMutations"
import { withOneShotUnlock } from "@/data/safety/oneShotUnlock"
import { useConnectionLockState } from "@/data/safety/queries"
import { useKeyboardShortcut } from "@/hooks/useKeyboardShortcut"
import type { Connection, QueueId } from "@/lib/tauri/bindings"
import { useBulkProgressStore } from "@/stores/useBulkProgressStore"
import { type JobSelectionEntry, useJobSelectionStore } from "@/stores/useJobSelectionStore"
import { CompleteView, ExecutingView, PreviewSheet, SelectingView } from "./BulkActionViews"
import { type BarState, bulkBarReducer, cancelledSummary, failedSummary } from "./bulkActionState"

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
 * One queue selected: one bulk call (`useBulkRetry` / `Remove` / `Promote`).
 * Several: one call per queue, run in turn (`use…CrossQueue`), and the
 * preview, progress and summary break the result down per queue.
 *
 * Absent when the backend does not declare bulk job actions.
 */

export interface BulkActionBarProps {
  connectionId: string
  queueId: QueueId
}

export function BulkActionBar({ connectionId, queueId }: BulkActionBarProps) {
  const selection = useJobSelectionStore((s) => s.selection)
  const distinctQueues = useJobSelectionStore((s) => s.distinctQueues)
  const groupByQueue = useJobSelectionStore((s) => s.groupByQueue)
  const clear = useJobSelectionStore((s) => s.clear)
  const [state, dispatch] = useReducer(bulkBarReducer, { kind: "selecting" } as BarState)

  // One queue: one bulk call.
  const retry = useBulkRetry()
  const remove = useBulkRemove()
  const promote = useBulkPromote()
  // Several queues: one call per queue, run in turn.
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

  // The connection, for its name on a protected confirmation.
  const { data: connections } = useConnections({ staleTime: 30_000 })
  const connection: Connection | undefined = connections?.find((c) => c.id === connectionId)

  const selectedCount = selection.size
  const queueCount = distinctQueues().size
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
        // One queue: one bulk call.
        const firstEntry: JobSelectionEntry[] | undefined = groups.values().next().value
        const jobIds = (firstEntry ?? []).map((e) => e.jobRef.id)
        const queueIdSingle = firstEntry?.[0]?.jobRef.queueId ?? queueId

        startProgress(new Map([[queueIdSingle, { action: kind, total: jobIds.length, jobIds }]]))
        const input = { connectionId, queueId: queueIdSingle, jobIds }
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
            dispatch({
              type: "complete",
              action: kind,
              result: failedSummary(kind, jobIds.length, message),
            })
          },
        })
        return
      }

      // Several queues: one call per queue, in turn.
      const perQueueInit = new Map<
        string,
        { action: BulkActionKind; total: number; jobIds: string[] }
      >()
      for (const [id, entries] of groups.entries()) {
        perQueueInit.set(id, {
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
        // No onError: the cross-queue orchestrator collects
        // per-queue errors into `result.perQueue[i].error` and
        // resolves the mutation. A top-level rejection here would
        // only fire if `aggregateCrossQueueResults` itself threw
        // (which it doesn't); leaving onError unhandled is safe.
      })
    },
    [
      groupByQueue,
      isCrossQueue,
      queueId,
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
  // The selected queues' names, by id, from their jobs.
  const queueNames = new Map(
    Array.from(groupByQueue().entries()).map(([id, entries]) => [
      id,
      entries[0]?.jobRef.queueName ?? id,
    ]),
  )
  const singleQueueName = Array.from(queueNames.values())[0] ?? queueId
  const protectedName = locked
    ? isCrossQueue
      ? (connection?.name ?? connectionId)
      : singleQueueName
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
      dispatch({
        type: "complete",
        action: state.action,
        result: cancelledSummary(state.action, {
          total: aggregateTotal(),
          completed: aggregateCompleted(),
          failedSoFar: aggregateFailedSoFar(),
        }),
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
  const bulk = useCapabilities(connectionId).can("jobActions.bulk")
  const selecting = bulk && state.kind === "selecting" && selectedCount > 0
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

  if (!bulk || (selectedCount === 0 && state.kind !== "complete")) {
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
      // The selection colour, like the selected rows: environment colours
      // belong to the environment dot and tag only.
      className="flex w-full items-center gap-3 border-b border-(--line) bg-(--sel) px-4 py-2 text-sm"
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
          singleQueueName={singleQueueName}
          queueNames={queueNames}
          onCancel={handleCancel}
        />
      )}
      {state.kind === "complete" && <CompleteView result={state.result} onDismiss={handleCancel} />}
    </section>
  )
}
