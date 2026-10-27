import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { listen } from "@tauri-apps/api/event"
import { useEffect } from "react"

import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { type ApprovalDecision, commands, type PendingApproval } from "@/lib/tauri/bindings"

export const PENDING_APPROVALS_QUERY_KEY = ["mcpPendingApprovals"] as const
/** Mirrors `PENDING_APPROVAL_EVENT` in `core/mcp/watcher.rs`. */
export const PENDING_APPROVAL_EVENT = "mcp:pending_approval"

/**
 * Agent requests waiting for a decision, oldest first.
 *
 * The backend watcher emits the full list whenever it changes; the initial
 * query covers requests that were already pending when the window opened.
 * A slow refetch is the safety net should an event be missed — a blocked agent
 * must never stay invisible.
 */
export function usePendingApprovals() {
  const queryClient = useQueryClient()
  // AI features off: nothing can wait (the requests were denied).
  const aiOn = useAiEnabled()

  const query = useQuery({
    enabled: aiOn,
    queryKey: PENDING_APPROVALS_QUERY_KEY,
    queryFn: async () => {
      const result = await commands.listPendingApprovals()
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
    refetchInterval: 5_000,
  })

  useEffect(() => {
    let unlisten: (() => void) | undefined
    let cancelled = false
    void listen<PendingApproval[]>(PENDING_APPROVAL_EVENT, (event) => {
      queryClient.setQueryData(PENDING_APPROVALS_QUERY_KEY, event.payload)
    })
      .then((fn) => {
        if (cancelled) fn()
        else unlisten = fn
      })
      .catch(() => {
        // Outside Tauri (tests, plain browser): the refetch interval carries on.
      })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [queryClient])

  return aiOn ? (query.data ?? NONE) : NONE
}

const NONE: PendingApproval[] = []

export function useDecideApproval() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (vars: {
      approvalId: string
      decision: ApprovalDecision
      reason?: string
    }) => {
      const result = await commands.decidePendingApproval({
        approvalId: vars.approvalId,
        decision: vars.decision,
        reason: vars.reason?.trim() ? vars.reason.trim() : null,
      })
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
    // Decided or `alreadyDecided` alike: it is no longer ours to show.
    onSuccess: (_outcome, vars) => {
      queryClient.setQueryData<PendingApproval[]>(PENDING_APPROVALS_QUERY_KEY, (old) =>
        old?.filter((a) => a.id !== vars.approvalId),
      )
    },
  })
}
