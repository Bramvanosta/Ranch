import { useQuery } from "@tanstack/react-query"
import { TriangleAlertIcon } from "lucide-react"

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { commands, type PingConnectionResult } from "@/lib/tauri/bindings"

interface ReachabilityIndicatorProps {
  connectionId: string
  /** The sidebar only says something when the server cannot be reached. */
  onlyProblems?: boolean
}

/**
 * per-connection reachability dot for the sidebar.
 *
 * Pings the connection every {@link REACHABILITY_POLL_MS} via TanStack Query.
 * Three render states:
 *   - Loading: muted dot, no text.
 *   - Reachable: green dot + latency in mono `font-tabular`.
 *   - Unreachable: muted warning icon with a Tooltip describing the failure.
 *
 * Could move from `useQuery` polling to a Tauri-event subscription
 * (`connection:reachability_changed`) later. The component API would stay
 * the same.
 *
 * Polling does NOT continue when the document is hidden — TanStack Query's
 * default for `refetchIntervalInBackground` is `false` and we don't override.
 */
export const REACHABILITY_POLL_MS = 30_000

export function ReachabilityIndicator({
  connectionId,
  onlyProblems = false,
}: ReachabilityIndicatorProps) {
  const { data, isLoading, isError, error } = useQuery({
    queryKey: ["ping", connectionId],
    queryFn: async () => {
      const r = await commands.pingConnection(connectionId)
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    refetchInterval: REACHABILITY_POLL_MS,
    refetchOnWindowFocus: true,
    // Don't retry — failures are valid signals (unreachable host) that we
    // want to surface quickly rather than mask with retries.
    retry: false,
  })

  if (onlyProblems && (isLoading || data?.success)) return null

  if (isLoading) {
    return (
      <span
        role="status"
        aria-label="Pinging…"
        className="inline-block h-2 w-2 rounded-full bg-(--text2)/40"
      />
    )
  }

  if (isError || !data) {
    const reason = error instanceof Error ? error.message : "Unknown error"
    return <UnreachableIcon reason={reason} />
  }

  if (data.success) {
    return (
      <span
        role="status"
        className="flex items-center gap-1"
        aria-label={`Reachable, ${data.latencyMs} ms`}
      >
        <span aria-hidden="true" className="h-2 w-2 rounded-full bg-(--state-completed)" />
        <span className="font-tabular text-2xs text-(--text2)">{data.latencyMs} ms</span>
      </span>
    )
  }

  return <UnreachableIcon reason={reachabilityErrorLabel(data)} />
}

function UnreachableIcon({ reason }: { reason: string }) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <span role="status" aria-label={`Unreachable: ${reason}`} className="inline-flex">
            <TriangleAlertIcon className="h-3.5 w-3.5 text-(--text2)" />
          </span>
        }
      />
      <TooltipContent>{reason}</TooltipContent>
    </Tooltip>
  )
}

function reachabilityErrorLabel(data: PingConnectionResult): string {
  if (!data.error) return "Cannot reach"
  switch (data.error.kind) {
    case "authFailed":
      return "Auth failed"
    case "cannotReach":
      return "Cannot reach"
    case "timeout":
      return "Timeout (>2s)"
    case "tlsHandshake":
      return "TLS handshake failed"
    case "sshTunnel":
      return `SSH tunnel: ${data.error.reason}`
    case "other":
      return data.error.reason
  }
}
