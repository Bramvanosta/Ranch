import { unwrap } from "@/data/result"
import { commands } from "@/lib/tauri/bindings"

/**
 * Run one confirmed destructive action on a protected
 * connection: unlock it, run, lock it again, whatever happens. Both steps
 * go through the lock commands, so the audit log records the unlock and
 * the re-lock around the action.
 */
export async function withOneShotUnlock<T>(
  connectionId: string,
  run: () => Promise<T>,
): Promise<T> {
  const state = await commands.getConnectionLockState({ connectionId })
  const wasLocked = state.status === "ok" && state.data === "locked"
  if (wasLocked) {
    unwrap(await commands.unlockConnection({ connectionId }))
  }
  try {
    return await run()
  } finally {
    if (wasLocked) await commands.lockConnection({ connectionId })
  }
}
