import { useQuery } from "@tanstack/react-query"

import { type CommandResult, unwrap } from "@/data/result"
import {
  type BackendInfo,
  type Connection,
  commands,
  type ParsedRedisUrl,
  type TestConnectionArgs,
  type TestConnectionResult,
} from "@/lib/tauri/bindings"

import { connectionKeys } from "./keys"

/** `list_connections`, for a query of `connectionKeys.all` or a one-off read. */
export async function fetchConnections(): Promise<Connection[]> {
  return unwrap(await commands.listConnections())
}

/**
 * Every saved connection, from the one `["connections"]` query all screens
 * share. `staleTime` lets a screen that only needs names skip refetching.
 */
export function useConnections(options: { enabled?: boolean; staleTime?: number } = {}) {
  return useQuery({ queryKey: connectionKeys.all, queryFn: fetchConnections, ...options })
}

/** One saved connection, from the shared `["connections"]` query. */
export function useConnection(connectionId: string | null | undefined): Connection | undefined {
  const { data } = useConnections({ enabled: Boolean(connectionId) })
  return data?.find((c) => c.id === connectionId)
}

/** The kinds of backend this Ranch serves; they do not change while it runs. */
export function useBackends() {
  return useQuery({
    queryKey: connectionKeys.backends,
    queryFn: async (): Promise<BackendInfo[]> => unwrap(await commands.listBackends()),
    staleTime: Number.POSITIVE_INFINITY,
  })
}

/** How often the sidebar asks whether a connection answers. */
export const REACHABILITY_POLL_MS = 30_000

/**
 * Whether a connection answers, and how fast, every
 * {@link REACHABILITY_POLL_MS}. TanStack Query pauses the poll while the
 * document is hidden (`refetchIntervalInBackground` stays `false`).
 */
export function usePingConnection(connectionId: string) {
  return useQuery({
    queryKey: connectionKeys.ping(connectionId),
    queryFn: async () => unwrap(await commands.pingConnection(connectionId)),
    refetchInterval: REACHABILITY_POLL_MS,
    refetchOnWindowFocus: true,
    // Don't retry: failures are valid signals (unreachable host) that we
    // want to surface quickly rather than mask with retries.
    retry: false,
  })
}

/** Split a `redis://` / `rediss://` URL into the form's fields. */
export function parseRedisUrl(raw: string): Promise<CommandResult<ParsedRedisUrl>> {
  return commands.parseRedisUrl(raw)
}

/** Try the form's settings without saving them. */
export function testConnection(
  args: TestConnectionArgs,
): Promise<CommandResult<TestConnectionResult>> {
  return commands.testConnection(args)
}
