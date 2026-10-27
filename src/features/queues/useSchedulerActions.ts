import { useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import {
  commands,
  type QueueRef,
  type SchedulerActionResult,
  type SchedulerSaved,
  type SchedulerSpec,
} from "@/lib/tauri/bindings"

/**
 * TanStack Query mutation hooks for pause/resume CRON
 * scheduler. Mirrors the shape of `useQueueActions` but operates on
 * a single scheduler (identified by `schedulerKey`).
 *
 * `schedulerNotFound` errors (race with concurrent removal) surface
 * a distinct toast — no special branching needed; the backend's
 * error message is human-friendly already.
 */
export interface SchedulerActionInput {
  connectionId: string
  queueRef: QueueRef
  schedulerKey: string
}

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

function invalidateAfterSchedulerAction(
  qc: ReturnType<typeof useQueryClient>,
  input: SchedulerActionInput,
) {
  qc.invalidateQueries({
    queryKey: ["cronJobs", input.connectionId, input.queueRef.prefix, input.queueRef.name],
  })
}

export function usePauseScheduler() {
  const qc = useQueryClient()
  return useMutation<SchedulerActionResult, Error, SchedulerActionInput>({
    mutationFn: async (input) => {
      const r = await commands.pauseScheduler({
        connectionId: input.connectionId,
        queueRef: input.queueRef,
        schedulerKey: input.schedulerKey,
      })
      throwWithKind(r)
      if (r.status !== "ok") throw new Error("unreachable")
      return r.data
    },
    onSuccess: (data) => {
      if (data.alreadyInState) {
        toast.success("Already paused", { duration: 2_000 })
      } else {
        toast.success("Scheduler paused", { duration: 2_000 })
      }
    },
    onError: (err) => {
      toast.error(buildErrorToast("Pause", err))
    },
    onSettled: (_d, _e, input) => invalidateAfterSchedulerAction(qc, input),
  })
}

export function useResumeScheduler() {
  const qc = useQueryClient()
  return useMutation<SchedulerActionResult, Error, SchedulerActionInput>({
    mutationFn: async (input) => {
      const r = await commands.resumeScheduler({
        connectionId: input.connectionId,
        queueRef: input.queueRef,
        schedulerKey: input.schedulerKey,
      })
      throwWithKind(r)
      if (r.status !== "ok") throw new Error("unreachable")
      return r.data
    },
    onSuccess: (data) => {
      if (data.alreadyInState) {
        toast.success("Already running", { duration: 2_000 })
      } else {
        toast.success("Scheduler resumed", { duration: 2_000 })
      }
    },
    onError: (err) => {
      toast.error(buildErrorToast("Resume", err))
    },
    onSettled: (_d, _e, input) => invalidateAfterSchedulerAction(qc, input),
  })
}

// ─────────────────────────────────────────────────────────────────────────
// Create / edit / delete
// ─────────────────────────────────────────────────────────────────────────

export interface UpsertSchedulerInput {
  connectionId: string
  queueRef: QueueRef
  scheduler: SchedulerSpec
}

/** Create or replace a scheduler. Errors are left to the sheet (shown in place). */
export function useUpsertScheduler() {
  const qc = useQueryClient()
  return useMutation<SchedulerSaved, Error, UpsertSchedulerInput>({
    mutationFn: async (input) => {
      const r = await commands.upsertScheduler(input)
      throwWithKind(r)
      if (r.status !== "ok") throw new Error("unreachable")
      return r.data
    },
    onSettled: (_d, _e, input) => {
      invalidateAfterSchedulerAction(qc, { ...input, schedulerKey: input.scheduler.id })
      // Its pending job appears in (or leaves) delayed / waiting.
      void qc.invalidateQueries({ queryKey: ["queues", input.connectionId] })
      void qc.invalidateQueries({ queryKey: ["jobs", input.connectionId] })
    },
  })
}

/** Delete a scheduler and its pending job. */
export function useRemoveScheduler() {
  const qc = useQueryClient()
  return useMutation<SchedulerActionResult, Error, SchedulerActionInput>({
    mutationFn: async (input) => {
      const r = await commands.removeScheduler({
        connectionId: input.connectionId,
        queueRef: input.queueRef,
        schedulerKey: input.schedulerKey,
      })
      throwWithKind(r)
      if (r.status !== "ok") throw new Error("unreachable")
      return r.data
    },
    onSettled: (_d, _e, input) => {
      invalidateAfterSchedulerAction(qc, input)
      void qc.invalidateQueries({ queryKey: ["queues", input.connectionId] })
      void qc.invalidateQueries({ queryKey: ["jobs", input.connectionId] })
    },
  })
}
