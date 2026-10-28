import { useMutation, useQueryClient } from "@tanstack/react-query"

import { connectionKeys } from "@/data/connections/keys"
import { type CommandResult, unwrap } from "@/data/result"
import { toastFailure } from "@/data/toasts"
import {
  type ChangeEnvironmentColorArgs,
  type CreateEnvironmentArgs,
  commands,
  type RenameEnvironmentArgs,
} from "@/lib/tauri/bindings"

import { environmentKeys } from "./keys"

/** A new environment in a group; answers its id. The caller refreshes the hierarchy. */
export function createEnvironment(args: CreateEnvironmentArgs): Promise<CommandResult<string>> {
  return commands.createEnvironment(args)
}

/** Rename an environment. The caller refreshes the hierarchy. */
export function renameEnvironment(args: RenameEnvironmentArgs): Promise<CommandResult<null>> {
  return commands.renameEnvironment(args)
}

/**
 * The colour belongs to the environment: recolouring applies to every
 * connection in it.
 */
export function useChangeEnvironmentColor() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (vars: ChangeEnvironmentColorArgs) => {
      unwrap(await commands.changeEnvironmentColor(vars))
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: connectionKeys.all })
      void queryClient.invalidateQueries({ queryKey: environmentKeys.all })
    },
    onError: (err) => toastFailure("Could not change color", err),
  })
}
