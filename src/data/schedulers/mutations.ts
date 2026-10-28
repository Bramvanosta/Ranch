import { useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import { jobKeys } from "@/data/jobs/keys"
import { queueKeys } from "@/data/queues/keys"
import { type CommandResult, unwrap } from "@/data/result"
import { actionFailedMessage } from "@/data/toasts"
import {
  commands,
  type QueueId,
  type SchedulerActionArgs,
  type SchedulerActionResult,
  type SchedulerSaved,
  type SchedulerSpec,
} from "@/lib/tauri/bindings"
import { schedulerKeys } from "./keys"

/**
 * TanStack Query mutation hooks for pause/resume CRON
 * scheduler. Mirrors the shape of `useQueueActions` but operates on
 * a single scheduler (identified by `schedulerKey`).
 *
 * `schedulerNotFound` errors (race with concurrent removal) surface
 * a distinct toast; no special branching needed; the backend's
 * error message is human-friendly already.
 */
export interface SchedulerActionInput {
  connectionId: string
  queueId: QueueId
  schedulerKey: string
}

function invalidateAfterSchedulerAction(
  qc: ReturnType<typeof useQueryClient>,
  input: SchedulerActionInput,
) {
  qc.invalidateQueries({
    queryKey: schedulerKeys.list(input.connectionId, input.queueId),
  })
}

export function usePauseScheduler() {
  const qc = useQueryClient()
  return useMutation<SchedulerActionResult, Error, SchedulerActionInput>({
    mutationFn: async (input) => {
      return unwrap(
        await commands.pauseScheduler({
          connectionId: input.connectionId,
          queueId: input.queueId,
          schedulerKey: input.schedulerKey,
        }),
      )
    },
    onSuccess: (data) => {
      if (data.alreadyInState) {
        toast.success("Already paused", { duration: 2_000 })
      } else {
        toast.success("Scheduler paused", { duration: 2_000 })
      }
    },
    onError: (err) => {
      toast.error(actionFailedMessage("Pause", err))
    },
    onSettled: (_d, _e, input) => invalidateAfterSchedulerAction(qc, input),
  })
}

export function useResumeScheduler() {
  const qc = useQueryClient()
  return useMutation<SchedulerActionResult, Error, SchedulerActionInput>({
    mutationFn: async (input) => {
      return unwrap(
        await commands.resumeScheduler({
          connectionId: input.connectionId,
          queueId: input.queueId,
          schedulerKey: input.schedulerKey,
        }),
      )
    },
    onSuccess: (data) => {
      if (data.alreadyInState) {
        toast.success("Already running", { duration: 2_000 })
      } else {
        toast.success("Scheduler resumed", { duration: 2_000 })
      }
    },
    onError: (err) => {
      toast.error(actionFailedMessage("Resume", err))
    },
    onSettled: (_d, _e, input) => invalidateAfterSchedulerAction(qc, input),
  })
}

// ─────────────────────────────────────────────────────────────────────────
// Create / edit / delete
// ─────────────────────────────────────────────────────────────────────────

export interface UpsertSchedulerInput {
  connectionId: string
  queueId: QueueId
  scheduler: SchedulerSpec
}

/** Create or replace a scheduler. Errors are left to the sheet (shown in place). */
export function useUpsertScheduler() {
  const qc = useQueryClient()
  return useMutation<SchedulerSaved, Error, UpsertSchedulerInput>({
    mutationFn: async (input) => {
      return unwrap(await commands.upsertScheduler(input))
    },
    onSettled: (_d, _e, input) => {
      invalidateAfterSchedulerAction(qc, { ...input, schedulerKey: input.scheduler.id })
      // Its pending job appears in (or leaves) delayed / waiting.
      void qc.invalidateQueries({ queryKey: queueKeys.list(input.connectionId) })
      void qc.invalidateQueries({ queryKey: jobKeys.all(input.connectionId) })
    },
  })
}

/** Delete a scheduler and its pending job. */
export function useRemoveScheduler() {
  const qc = useQueryClient()
  return useMutation<SchedulerActionResult, Error, SchedulerActionInput>({
    mutationFn: async (input) => {
      return unwrap(
        await commands.removeScheduler({
          connectionId: input.connectionId,
          queueId: input.queueId,
          schedulerKey: input.schedulerKey,
        }),
      )
    },
    onSettled: (_d, _e, input) => {
      invalidateAfterSchedulerAction(qc, input)
      void qc.invalidateQueries({ queryKey: queueKeys.list(input.connectionId) })
      void qc.invalidateQueries({ queryKey: jobKeys.all(input.connectionId) })
    },
  })
}

/** `remove_scheduler` as a plain call, for a flow that runs it under a one-shot unlock. */
export function removeScheduler(
  args: SchedulerActionArgs,
): Promise<CommandResult<SchedulerActionResult>> {
  return commands.removeScheduler(args)
}
