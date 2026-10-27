import { useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import type { HoldsPoll } from "@/lib/polling/sharedPoll"
import { commands, type Queue, type QueueActionResult, type QueueRef } from "@/lib/tauri/bindings"

/**
 * TanStack Query mutation hooks for pause/resume queue.
 *
 * Mirrors the shape of `useJobActions`:
 *   - Errors preserve `kind` on the thrown Error so callers can
 *     branch on `connectionLocked` (toast renders the verbatim
 *     backend message — it already includes the unlock keystroke).
 *   - 2s success toast.
 *
 * On a slow link "the 1 s poll is fast enough" no longer held:
 *   - the queue flips to paused / running at once (optimistic), and flips
 *     back with the reason if the action fails;
 *   - the queue list's poll waits while the action runs (`holdsPoll`), so a
 *     read sent before the change cannot put the old state back (the
 *     flicker the earlier design avoided by not flipping);
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
  queueRef: QueueRef
}

interface QueueActionContext {
  previous?: Queue[]
}

const queuesKey = (connectionId: string) => ["queues", connectionId]

const holdsPoll: HoldsPoll = (variables) => queuesKey((variables as QueueActionInput).connectionId)

function throwWithKind(r: {
  status: "ok" | "error"
  data?: unknown
  error?: { kind: string; message: string }
}) {
  if (r.status === "error" && r.error) {
    const err = new Error(r.error.message)
    ;(err as Error & { kind?: string }).kind = r.error.kind
    throw err
  }
}

function errorKind(err: unknown): string | undefined {
  if (err && typeof err === "object" && "kind" in err) {
    return (err as { kind?: string }).kind
  }
  return undefined
}

function buildErrorToast(action: string, err: unknown): string {
  const kind = errorKind(err)
  const msg = err instanceof Error ? err.message : String(err)
  if (kind === "connectionLocked") {
    return msg
  }
  return `${action} failed: ${msg}`
}

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
    queues?.map((q) =>
      q.queueRef.prefix === input.queueRef.prefix && q.queueRef.name === input.queueRef.name
        ? { ...q, isPaused }
        : q,
    ),
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
      const r = await commands.pauseQueue({
        connectionId: input.connectionId,
        queueRef: input.queueRef,
      })
      throwWithKind(r)
      if (r.status !== "ok") throw new Error("unreachable")
      return r.data
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
      toast.error(buildErrorToast("Pause", err))
    },
    onSettled: (_d, _e, input) => refresh(qc, input),
  })
}

export function useResumeQueue() {
  const qc = useQueryClient()
  return useMutation<QueueActionResult, Error, QueueActionInput, QueueActionContext>({
    meta: { holdsPoll },
    mutationFn: async (input) => {
      const r = await commands.resumeQueue({
        connectionId: input.connectionId,
        queueRef: input.queueRef,
      })
      throwWithKind(r)
      if (r.status !== "ok") throw new Error("unreachable")
      return r.data
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
      toast.error(buildErrorToast("Resume", err))
    },
    onSettled: (_d, _e, input) => refresh(qc, input),
  })
}
