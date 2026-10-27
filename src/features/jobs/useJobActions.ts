import { type QueryKey, useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import type { HoldsPoll } from "@/lib/polling/sharedPoll"
import {
  commands,
  type Job,
  type JobActionResult,
  type JobState,
  type ListJobsOutput,
  type QueueRef,
} from "@/lib/tauri/bindings"

/**
 * single-job destructive mutations
 * (retry / remove / promote).
 *
 * All three hooks share the same shape:
 *   - Optimistic update via `onMutate` so the affected row visually
 *     transitions immediately.
 *   - Cache rollback on error via the `prev` snapshot stored in
 *     `onMutate`'s return value.
 *   - `connectionLocked` errors surface the unlock keystroke inline
 *     in the toast; the user discovers the lock at the
 *     moment of action, not by the absence of affordances.
 *   - 2s success toast.
 *   - `onSettled` invalidates the relevant queries so the next poll
 *     reflects truth.
 *
 * For `useRemoveJob`, the optimistic update sets `state = waiting`
 * (and removes the job from the list cache when present). The
 * "retrying" / "removing" / "promoting" affordance is owned by the
 * row component checking `mutation.isPending` against the row's id.
 */

export interface JobActionInput {
  connectionId: string
  queueRef: QueueRef
  jobId: string
}

/**
 * While an action runs, the queue's job lists are not polled:
 * a read sent before it landed would put the row back (see `holdsPoll`).
 */
const jobListsOf: HoldsPoll = (variables) => {
  const input = variables as JobActionInput
  return ["jobs", input.connectionId, input.queueRef.prefix, input.queueRef.name]
}

interface MutationContext {
  prevJob?: Job
  /** The job lists as they were, to put the row back. */
  prevLists?: Array<[QueryKey, unknown]>
}

/**
 * The row leaves the queue's lists at once (a retried job
 * leaves Failed, a removed one every list, a promoted one Delayed), and
 * comes back if the action fails. Lists of `keepState` keep it: a retried
 * job is now waiting, and the next read places it.
 */
async function optimisticallyDropFromLists(
  qc: ReturnType<typeof useQueryClient>,
  input: JobActionInput,
  keepState: JobState | null,
): Promise<Array<[QueryKey, unknown]>> {
  const filter = {
    queryKey: ["jobs", input.connectionId, input.queueRef.prefix, input.queueRef.name],
    predicate: (q: { queryKey: QueryKey }) => keepState === null || q.queryKey[4] !== keepState,
  }
  await qc.cancelQueries(filter)
  const prev = qc.getQueriesData(filter)
  qc.setQueriesData<ListJobsOutput>(filter, (list) => {
    if (!list?.jobs.some((j) => j.jobRef.id === input.jobId)) return list
    return {
      ...list,
      jobs: list.jobs.filter((j) => j.jobRef.id !== input.jobId),
      total: Math.max(0, list.total - 1),
    }
  })
  return prev
}

/**
 * Helper — best-effort optimistic state flip in the `["job", ...]`
 * cache. Returns the previous job for rollback. Returns `undefined`
 * if no cached entry exists (the user hasn't drilled in).
 */
function optimisticallyFlipState(
  qc: ReturnType<typeof useQueryClient>,
  input: JobActionInput,
  newState: JobState,
): MutationContext {
  const key = ["job", input.connectionId, input.queueRef.prefix, input.queueRef.name, input.jobId]
  const prev = qc.getQueryData<Job>(key)
  if (prev) {
    qc.setQueryData<Job>(key, { ...prev, state: newState })
  }
  return { prevJob: prev }
}

function rollback(
  qc: ReturnType<typeof useQueryClient>,
  input: JobActionInput,
  ctx: MutationContext | undefined,
) {
  for (const [key, data] of ctx?.prevLists ?? []) qc.setQueryData(key, data)
  if (!ctx?.prevJob) return
  const key = ["job", input.connectionId, input.queueRef.prefix, input.queueRef.name, input.jobId]
  qc.setQueryData<Job>(key, ctx.prevJob)
}

/**
 * Returned to `onSettled`, so the mutation stays pending (its
 * control busy) until the refreshed data is back; refetches in flight are
 * left to finish rather than cancelled and sent again.
 */
function invalidateAfterAction(qc: ReturnType<typeof useQueryClient>, input: JobActionInput) {
  const keep = { cancelRefetch: false }
  return Promise.all([
    // Drop all `list_jobs` pages for the queue (state-filtered cache).
    qc.invalidateQueries(
      { queryKey: ["jobs", input.connectionId, input.queueRef.prefix, input.queueRef.name] },
      keep,
    ),
    // Refetch the single-job cache so the drill-down view picks up truth.
    qc.invalidateQueries(
      {
        queryKey: [
          "job",
          input.connectionId,
          input.queueRef.prefix,
          input.queueRef.name,
          input.jobId,
        ],
      },
      keep,
    ),
    // Drop the queue counter cache so the JobStateChips update.
    qc.invalidateQueries({ queryKey: ["queues", input.connectionId] }, keep),
  ])
}

/**
 * Extract the `kind` discriminator from a Tauri error so the hook
 * can branch on it (specifically for `connectionLocked`, which
 * includes the unlock keystroke in the message body).
 */
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
    // The backend already concatenated the unlock hint into the
    // message body — render verbatim.
    return msg
  }
  return `${action} failed: ${msg}`
}

// ─────────────────────────────────────────────────────────────────────────
// useRetryJob
// ─────────────────────────────────────────────────────────────────────────

export function useRetryJob() {
  const qc = useQueryClient()
  return useMutation<JobActionResult, Error, JobActionInput, MutationContext>({
    meta: { holdsPoll: jobListsOf },
    mutationFn: async (input) => {
      const r = await commands.retryJob({
        connectionId: input.connectionId,
        queueRef: input.queueRef,
        jobId: input.jobId,
      })
      if (r.status === "error") {
        const err = new Error(r.error.message)
        ;(err as Error & { kind?: string }).kind = r.error.kind
        throw err
      }
      return r.data
    },
    onMutate: async (input) => {
      await qc.cancelQueries({
        queryKey: [
          "job",
          input.connectionId,
          input.queueRef.prefix,
          input.queueRef.name,
          input.jobId,
        ],
      })
      return {
        ...optimisticallyFlipState(qc, input, "waiting"),
        prevLists: await optimisticallyDropFromLists(qc, input, "waiting"),
      }
    },
    onError: (err, input, ctx) => {
      rollback(qc, input, ctx)
      toast.error(buildErrorToast("Retry", err))
    },
    onSuccess: () => {
      toast.success("Retried", { duration: 2_000 })
    },
    onSettled: (_d, _e, input) => invalidateAfterAction(qc, input),
  })
}

// ─────────────────────────────────────────────────────────────────────────
// useRemoveJob
// ─────────────────────────────────────────────────────────────────────────

export function useRemoveJob() {
  const qc = useQueryClient()
  return useMutation<JobActionResult, Error, JobActionInput, MutationContext>({
    meta: { holdsPoll: jobListsOf },
    mutationFn: async (input) => {
      const r = await commands.removeJob({
        connectionId: input.connectionId,
        queueRef: input.queueRef,
        jobId: input.jobId,
      })
      if (r.status === "error") {
        const err = new Error(r.error.message)
        ;(err as Error & { kind?: string }).kind = r.error.kind
        throw err
      }
      return r.data
    },
    onMutate: async (input) => {
      await qc.cancelQueries({
        queryKey: [
          "job",
          input.connectionId,
          input.queueRef.prefix,
          input.queueRef.name,
          input.jobId,
        ],
      })
      // Snapshot the cached job for rollback; we don't optimistically
      // delete the cache entry here because rollback-then-restore is
      // simpler than re-inserting a deleted entry on error.
      const key = [
        "job",
        input.connectionId,
        input.queueRef.prefix,
        input.queueRef.name,
        input.jobId,
      ]
      const prev = qc.getQueryData<Job>(key)
      return { prevJob: prev, prevLists: await optimisticallyDropFromLists(qc, input, null) }
    },
    onError: (err, input, ctx) => {
      rollback(qc, input, ctx)
      toast.error(buildErrorToast("Remove", err))
    },
    onSuccess: () => {
      toast.success("Removed", { duration: 2_000 })
    },
    onSettled: (_d, _e, input) => invalidateAfterAction(qc, input),
  })
}

// ─────────────────────────────────────────────────────────────────────────
// usePromoteJob
// ─────────────────────────────────────────────────────────────────────────

export function usePromoteJob() {
  const qc = useQueryClient()
  return useMutation<JobActionResult, Error, JobActionInput, MutationContext>({
    meta: { holdsPoll: jobListsOf },
    mutationFn: async (input) => {
      const r = await commands.promoteJob({
        connectionId: input.connectionId,
        queueRef: input.queueRef,
        jobId: input.jobId,
      })
      if (r.status === "error") {
        const err = new Error(r.error.message)
        ;(err as Error & { kind?: string }).kind = r.error.kind
        throw err
      }
      return r.data
    },
    onMutate: async (input) => {
      await qc.cancelQueries({
        queryKey: [
          "job",
          input.connectionId,
          input.queueRef.prefix,
          input.queueRef.name,
          input.jobId,
        ],
      })
      return {
        ...optimisticallyFlipState(qc, input, "waiting"),
        prevLists: await optimisticallyDropFromLists(qc, input, "waiting"),
      }
    },
    onError: (err, input, ctx) => {
      rollback(qc, input, ctx)
      toast.error(buildErrorToast("Promote", err))
    },
    onSuccess: () => {
      toast.success("Promoted", { duration: 2_000 })
    },
    onSettled: (_d, _e, input) => invalidateAfterAction(qc, input),
  })
}
