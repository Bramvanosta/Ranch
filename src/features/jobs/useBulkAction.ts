import { useMutation, useQueryClient } from "@tanstack/react-query"

import { type BulkActionResult, commands, type QueueRef } from "@/lib/tauri/bindings"

/**
 * TanStack Query mutation hooks for the 3 bulk actions.
 *
 * Each hook calls the matching `commands.bulkXxxJobs` Tauri command,
 * which dispatches via the connection actor (ONE lock check, ONE
 * consolidated audit row). The actor emits `bulk:progress` events
 * during the loop; the global event listener (mounted in App.tsx)
 * feeds those into `useBulkProgressStore` so the UI updates live.
 *
 * On success: invalidates `["jobs", ...]` and `["queues", ...]` so
 * the next poll picks up the post-bulk state. No optimistic per-row
 * flips — the BulkActionBar shows the consolidated summary; per-row
 * visuals come from the progress store.
 *
 * On error (notably `connectionLocked`): the error's `kind` is
 * preserved on the thrown Error via `(err as Error & { kind?: string }).kind`
 * so the `BulkActionBar` can synthesize a "consolidated locked
 * summary" view.
 */
export interface BulkActionInput {
  connectionId: string
  queueRef: QueueRef
  jobIds: string[]
}

function invalidateAfterBulk(qc: ReturnType<typeof useQueryClient>, input: BulkActionInput) {
  qc.invalidateQueries({
    queryKey: ["jobs", input.connectionId, input.queueRef.prefix, input.queueRef.name],
  })
  qc.invalidateQueries({ queryKey: ["queues", input.connectionId] })
  // Invalidate any cached single-job entries for the affected job ids.
  for (const jobId of input.jobIds) {
    qc.invalidateQueries({
      queryKey: ["job", input.connectionId, input.queueRef.prefix, input.queueRef.name, jobId],
    })
  }
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

export function useBulkRetry() {
  const qc = useQueryClient()
  return useMutation<BulkActionResult, Error, BulkActionInput>({
    mutationFn: async (input) => {
      const r = await commands.bulkRetryJobs({
        connectionId: input.connectionId,
        queueRef: input.queueRef,
        jobIds: input.jobIds,
      })
      throwWithKind(r)
      // Narrow after throwWithKind returns without throwing.
      if (r.status !== "ok") throw new Error("unreachable")
      return r.data
    },
    onSettled: (_d, _e, input) => invalidateAfterBulk(qc, input),
  })
}

export function useBulkRemove() {
  const qc = useQueryClient()
  return useMutation<BulkActionResult, Error, BulkActionInput>({
    mutationFn: async (input) => {
      const r = await commands.bulkRemoveJobs({
        connectionId: input.connectionId,
        queueRef: input.queueRef,
        jobIds: input.jobIds,
      })
      throwWithKind(r)
      if (r.status !== "ok") throw new Error("unreachable")
      return r.data
    },
    onSettled: (_d, _e, input) => invalidateAfterBulk(qc, input),
  })
}

export function useBulkPromote() {
  const qc = useQueryClient()
  return useMutation<BulkActionResult, Error, BulkActionInput>({
    mutationFn: async (input) => {
      const r = await commands.bulkPromoteJobs({
        connectionId: input.connectionId,
        queueRef: input.queueRef,
        jobIds: input.jobIds,
      })
      throwWithKind(r)
      if (r.status !== "ok") throw new Error("unreachable")
      return r.data
    },
    onSettled: (_d, _e, input) => invalidateAfterBulk(qc, input),
  })
}
