import { useQuery } from "@tanstack/react-query"

import { type CommandResult, unwrap } from "@/data/result"
import { commands, type ReadOnlyState } from "@/lib/tauri/bindings"

import { safetyKeys } from "./keys"

/**
 * The per-connection session lock (`locked` / `unlocked`).
 *
 * The state is owned by the connection's backend (in memory; for BullMQ, the
 * connection actor's `ActorState.lock_state`). The first read
 * (`get_connection_lock_state`) lazy-spawns the actor, which starts from the
 * `connections.is_locked` column. `staleTime: Infinity` because the state
 * only changes through the lock mutations; there's no background source
 * that can flip it.
 *
 * A restart re-spawns the actor and re-reads `is_locked`, so unlocks are
 * session-bound: on next launch the first read answers `locked` again for
 * production connections.
 */
export function useConnectionLockState(connectionId: string, { enabled = true } = {}) {
  return useQuery({
    enabled,
    queryKey: safetyKeys.lockState(connectionId),
    queryFn: async () => unwrap(await commands.getConnectionLockState({ connectionId })),
    staleTime: Number.POSITIVE_INFINITY,
  })
}

/** The lock state as a plain call, for a confirm sheet that asks right before it opens. */
export function getConnectionLockState(
  connectionId: string,
): Promise<CommandResult<ReadOnlyState>> {
  return commands.getConnectionLockState({ connectionId })
}
