import { useQuery, useQueryClient } from "@tanstack/react-query"
import { listen } from "@tauri-apps/api/event"
import { useEffect } from "react"
import { unwrap } from "@/data/result"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { type ActivityFilter, commands, type PendingApproval } from "@/lib/tauri/bindings"
import { mcpKeys } from "./keys"

export const ACTIVITY_PAGE_SIZE = 100

export const EMPTY_FILTER: ActivityFilter = {
  from: null,
  to: null,
  clientHint: null,
  connectionId: null,
  kind: null,
  status: null,
  sessionId: null,
}

/**
 * What agents did. `ranch-mcp` is another process, so
 * there is no push: the panel refetches every few seconds while it is open.
 */
export function useMcpActivity(filter: ActivityFilter, page: number) {
  return useQuery({
    queryKey: mcpKeys.activityPage(filter, page),
    queryFn: async () => {
      return unwrap(
        await commands.listMcpActivity({
          filter,
          limit: ACTIVITY_PAGE_SIZE,
          offset: page * ACTIVITY_PAGE_SIZE,
        }),
      )
    },
    refetchInterval: 5_000,
    placeholderData: (previous) => previous,
  })
}

export function useMcpActivitySessions(enabled: boolean) {
  return useQuery({
    queryKey: mcpKeys.sessions,
    queryFn: async () => {
      return unwrap(await commands.listMcpActivitySessions())
    },
    enabled,
    refetchInterval: 5_000,
  })
}

/** The latest agent calls on one connection, for its live events. */
export function useConnectionAgentActivity(connectionId: string, enabled: boolean) {
  return useQuery({
    queryKey: mcpKeys.connectionActivity(connectionId),
    queryFn: async () => {
      const r = unwrap(
        await commands.listMcpActivity({
          filter: { ...EMPTY_FILTER, connectionId },
          limit: 20,
          offset: 0,
        }),
      )
      return r.events
    },
    enabled,
    refetchInterval: 5_000,
    refetchIntervalInBackground: false,
    retry: false,
  })
}

/** Mirrors `PENDING_APPROVAL_EVENT` in `core/mcp/watcher.rs`. */
export const PENDING_APPROVAL_EVENT = "mcp:pending_approval"

/**
 * Agent requests waiting for a decision, oldest first.
 *
 * The backend watcher emits the full list whenever it changes; the initial
 * query covers requests that were already pending when the window opened.
 * A slow refetch is the safety net should an event be missed: a blocked agent
 * must never stay invisible.
 */
export function usePendingApprovals() {
  const queryClient = useQueryClient()
  // AI features off: nothing can wait (the requests were denied).
  const aiOn = useAiEnabled()

  const query = useQuery({
    enabled: aiOn,
    queryKey: mcpKeys.pendingApprovals,
    queryFn: async () => {
      return unwrap(await commands.listPendingApprovals())
    },
    refetchInterval: 5_000,
  })

  useEffect(() => {
    let unlisten: (() => void) | undefined
    let cancelled = false
    void listen<PendingApproval[]>(PENDING_APPROVAL_EVENT, (event) => {
      queryClient.setQueryData(mcpKeys.pendingApprovals, event.payload)
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

/** The earlier decisions on agent requests, newest first. */
export function useApprovalHistory() {
  return useQuery({
    queryKey: mcpKeys.approvalHistory,
    queryFn: async () => {
      return unwrap(await commands.listApprovalHistory(50))
    },
    refetchInterval: 10_000,
  })
}

/** The approval behind an agent write, when there is one. */
export function useApprovalRecord(approvalId: string | null) {
  return useQuery({
    queryKey: mcpKeys.approvalRecord(approvalId),
    queryFn: async () => {
      if (!approvalId) return null
      return unwrap(await commands.getApprovalRecord(approvalId))
    },
    enabled: Boolean(approvalId),
    retry: false,
  })
}

/**
 * Global MCP safety settings. UI-only on purpose:
 * these commands are on the backend's MCP deny-list, an agent can never
 * reach them.
 */
export function useMcpSafetySettings() {
  return useQuery({
    queryKey: mcpKeys.safety,
    queryFn: async () => {
      return unwrap(await commands.getMcpSafetySettings())
    },
  })
}

/** Which agent clients are installed on this machine, and how each one is set up. */
export function useMcpInstallerStatus(enabled = true) {
  return useQuery({
    queryKey: mcpKeys.installerStatus,
    queryFn: async () => {
      return unwrap(await commands.getMcpInstallerStatus())
    },
    enabled,
  })
}

/** The configuration to paste by hand into a client Ranch cannot set up. */
export function useMcpManualSnippet() {
  return useQuery({
    queryKey: mcpKeys.manualSnippet,
    queryFn: async () => {
      return unwrap(await commands.getMcpManualSnippet())
    },
  })
}
