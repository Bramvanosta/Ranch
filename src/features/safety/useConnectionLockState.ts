import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import { commands, type ReadOnlyState } from "@/lib/tauri/bindings"

/**
 * TanStack Query hook for the per-connection read-only
 * lock state (`Locked` / `Unlocked`).
 *
 * The state is owned by the connection actor (in-memory, see
 * `core::connection_actor::actor::ActorState.lock_state`). On mount we
 * fetch the actor's current state via `get_connection_lock_state`,
 * which lazy-spawns the actor and initializes it from the
 * `connections.is_locked` column. `staleTime: Infinity` because the
 * state only changes through this hook's mutations — there's no
 * background source that can flip it.
 *
 * Restart of the app re-spawns the actor and re-reads `is_locked`, so
 * unlocks are session-bound. The frontend doesn't need to
 * model that explicitly — on next launch the initial fetch returns
 * `Locked` again for production connections.
 */
export const LOCK_STATE_QUERY_KEY = (connectionId: string): ["lockState", string] => [
  "lockState",
  connectionId,
]

export function useConnectionLockState(connectionId: string, { enabled = true } = {}) {
  return useQuery({
    enabled,
    queryKey: LOCK_STATE_QUERY_KEY(connectionId),
    queryFn: async () => {
      const r = await commands.getConnectionLockState({ connectionId })
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    staleTime: Number.POSITIVE_INFINITY,
  })
}

/**
 * Mutation hook that fires `unlock_connection`.
 *
 * Sets the actor's `lock_state = Unlocked` and writes an
 * `unlockConnection` audit entry. On success we
 * `setQueryData` the new state immediately so the banner flips on the
 * same render, and toast briefly so the user knows the gesture took.
 */
export function useUnlockConnection() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (
      connectionId: string,
    ): Promise<{ connectionId: string; state: ReadOnlyState }> => {
      const r = await commands.unlockConnection({ connectionId })
      if (r.status === "error") throw new Error(r.error.message)
      return { connectionId, state: r.data }
    },
    onSuccess: ({ connectionId, state }) => {
      qc.setQueryData(LOCK_STATE_QUERY_KEY(connectionId), state)
      toast.success("Unlocked for this session", { duration: 2_000 })
    },
    onError: (err: unknown) => {
      toast.error(`Could not unlock: ${err instanceof Error ? err.message : "unknown error"}`)
    },
  })
}

/**
 * Mutation hook that fires `lock_connection`.
 *
 * Symmetric to `useUnlockConnection`. Re-locks a previously-unlocked
 * connection for the rest of the session (or until the user unlocks
 * again).
 */
export function useLockConnection() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (
      connectionId: string,
    ): Promise<{ connectionId: string; state: ReadOnlyState }> => {
      const r = await commands.lockConnection({ connectionId })
      if (r.status === "error") throw new Error(r.error.message)
      return { connectionId, state: r.data }
    },
    onSuccess: ({ connectionId, state }) => {
      qc.setQueryData(LOCK_STATE_QUERY_KEY(connectionId), state)
      toast.success("Re-locked", { duration: 2_000 })
    },
    onError: (err: unknown) => {
      toast.error(`Could not re-lock: ${err instanceof Error ? err.message : "unknown error"}`)
    },
  })
}
