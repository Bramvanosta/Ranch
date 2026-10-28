import { useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import { jobKeys } from "@/data/jobs/keys"
import { type CommandResult, unwrap } from "@/data/result"
import { safetyKeys } from "@/data/safety/keys"
import { actionFailedMessage } from "@/data/toasts"
import type { HoldsPoll } from "@/lib/polling/sharedPoll"
import {
  commands,
  type MaintenanceResult,
  type Queue,
  type QueueActionArgs,
  type QueueActionResult,
  type QueueId,
  type QueueMaintenance,
} from "@/lib/tauri/bindings"
import { queueKeys } from "./keys"

/**
 * TanStack Query mutation hooks for pause/resume queue.
 *
 * Mirrors the shape of `useJobActions`:
 *   - Errors preserve `kind` on the thrown Error so callers can
 *     branch on `connectionLocked` (toast renders the verbatim
 *     backend message, which already includes the unlock keystroke).
 *   - 2s success toast.
 *
 * On a slow link the 1 s poll is not fast enough:
 *   - the queue flips to paused / running at once (optimistic), and flips
 *     back with the reason if the action fails;
 *   - the queue list's poll waits while the action runs (`holdsPoll`), so a
 *     read sent before the change cannot put the old state back;
 *   - `onSettled` returns the refresh, so the button stays busy until the
 *     refreshed list confirms it.
 *
 * Idempotency note: a successful no-op (the queue was already
 * in the target state) returns `{ isPaused, alreadyInState: true }`.
 * Toasted as "Already paused" / "Already running" so the user gets
 * feedback that the action was acknowledged.
 */
export interface QueueActionInput {
  connectionId: string
  queueId: QueueId
}

interface QueueActionContext {
  previous?: Queue[]
}

const queuesKey = (connectionId: string) => queueKeys.list(connectionId)

const holdsPoll: HoldsPoll = (variables) => queuesKey((variables as QueueActionInput).connectionId)

/** Set the queue's pause flag in the cached list; returns the list as it was. */
async function flip(
  qc: ReturnType<typeof useQueryClient>,
  input: QueueActionInput,
  isPaused: boolean,
): Promise<QueueActionContext> {
  const key = queuesKey(input.connectionId)
  await qc.cancelQueries({ queryKey: key })
  const previous = qc.getQueryData<Queue[]>(key)
  qc.setQueryData<Queue[]>(key, (queues) =>
    queues?.map((q) => (q.id === input.queueId ? { ...q, isPaused } : q)),
  )
  return { previous }
}

function rollback(
  qc: ReturnType<typeof useQueryClient>,
  input: QueueActionInput,
  ctx: QueueActionContext | undefined,
) {
  if (ctx?.previous) qc.setQueryData(queuesKey(input.connectionId), ctx.previous)
}

const refresh = (qc: ReturnType<typeof useQueryClient>, input: QueueActionInput) =>
  qc.invalidateQueries({ queryKey: queuesKey(input.connectionId) }, { cancelRefetch: false })

export function usePauseQueue() {
  const qc = useQueryClient()
  return useMutation<QueueActionResult, Error, QueueActionInput, QueueActionContext>({
    meta: { holdsPoll },
    mutationFn: async (input) => {
      return unwrap(
        await commands.pauseQueue({
          connectionId: input.connectionId,
          queueId: input.queueId,
        }),
      )
    },
    onMutate: (input) => flip(qc, input, true),
    onSuccess: (data) => {
      if (data.alreadyInState) {
        toast.success("Already paused", { duration: 2_000 })
      } else {
        toast.success("Queue paused", { duration: 2_000 })
      }
    },
    onError: (err, input, ctx) => {
      rollback(qc, input, ctx)
      toast.error(actionFailedMessage("Pause", err))
    },
    onSettled: (_d, _e, input) => refresh(qc, input),
  })
}

export function useResumeQueue() {
  const qc = useQueryClient()
  return useMutation<QueueActionResult, Error, QueueActionInput, QueueActionContext>({
    meta: { holdsPoll },
    mutationFn: async (input) => {
      return unwrap(
        await commands.resumeQueue({
          connectionId: input.connectionId,
          queueId: input.queueId,
        }),
      )
    },
    onMutate: (input) => flip(qc, input, false),
    onSuccess: (data) => {
      if (data.alreadyInState) {
        toast.success("Already running", { duration: 2_000 })
      } else {
        toast.success("Queue resumed", { duration: 2_000 })
      }
    },
    onError: (err, input, ctx) => {
      rollback(qc, input, ctx)
      toast.error(actionFailedMessage("Resume", err))
    },
    onSettled: (_d, _e, input) => refresh(qc, input),
  })
}

/** `resume_queue` as a plain call, for a flow that runs it under a one-shot unlock. */
export function resumeQueue(args: QueueActionArgs): Promise<CommandResult<QueueActionResult>> {
  return commands.resumeQueue(args)
}

/**
 * Drain, clean or retry every failed job of a queue. The actor works in
 * batches and reports progress through `bulk:progress`.
 */
export function runQueueMaintenance(
  connectionId: string,
  queueId: QueueId,
  operation: QueueMaintenance,
): Promise<CommandResult<MaintenanceResult>> {
  switch (operation.kind) {
    case "drain":
      return commands.drainQueue({
        connectionId,
        queueId,
        includeDelayed: operation.includeDelayed,
      })
    case "clean":
      return commands.cleanQueue({
        connectionId,
        queueId,
        state: operation.state,
        olderThanMs: operation.olderThanMs,
      })
    case "retryAllFailed":
      return commands.retryAllFailed({ connectionId, queueId })
  }
}

/**
 * After a maintenance operation, whatever its outcome: the lock (a one-shot
 * unlock locked it again), the job lists, the counters and the failed list.
 */
export function useInvalidateAfterMaintenance() {
  const qc = useQueryClient()
  return (connectionId: string) => {
    void qc.invalidateQueries({ queryKey: safetyKeys.lockState(connectionId) })
    void qc.invalidateQueries({ queryKey: jobKeys.all(connectionId) })
    void qc.invalidateQueries({ queryKey: queueKeys.list(connectionId) })
    void qc.invalidateQueries({ queryKey: jobKeys.failed(connectionId) })
  }
}
