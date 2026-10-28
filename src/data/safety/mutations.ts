import { useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import { unwrap } from "@/data/result"
import { toastFailure } from "@/data/toasts"
import { commands, type ReadOnlyState } from "@/lib/tauri/bindings"

import { safetyKeys } from "./keys"

/**
 * `unlock_connection`: the actor's lock state becomes `unlocked` and an
 * `unlockConnection` audit entry is written. The new state goes into the
 * cache at once so the banner flips on the same render, with a short toast
 * so the user knows the gesture took.
 */
export function useUnlockConnection() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (
      connectionId: string,
    ): Promise<{ connectionId: string; state: ReadOnlyState }> => {
      const state = unwrap(await commands.unlockConnection({ connectionId }))
      return { connectionId, state }
    },
    onSuccess: ({ connectionId, state }) => {
      qc.setQueryData(safetyKeys.lockState(connectionId), state)
      toast.success("Unlocked for this session", { duration: 2_000 })
    },
    onError: (err: unknown) => toastFailure("Could not unlock", err),
  })
}

/**
 * `lock_connection`, the other way: re-locks a connection for the rest of
 * the session (or until the user unlocks it again).
 */
export function useLockConnection() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (
      connectionId: string,
    ): Promise<{ connectionId: string; state: ReadOnlyState }> => {
      const state = unwrap(await commands.lockConnection({ connectionId }))
      return { connectionId, state }
    },
    onSuccess: ({ connectionId, state }) => {
      qc.setQueryData(safetyKeys.lockState(connectionId), state)
      toast.success("Re-locked", { duration: 2_000 })
    },
    onError: (err: unknown) => toastFailure("Could not re-lock", err),
  })
}
