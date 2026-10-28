import { toast } from "sonner"

import { errorKind } from "./result"

/** The message of a thrown value, for a toast. */
export function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : "unknown error"
}

/** "Could not remove: Connection refused": what a failed write tells the user. */
export function toastFailure(what: string, error: unknown) {
  toast.error(`${what}: ${messageOf(error)}`)
}

/**
 * The toast of a failed queue or job action. A `connectionLocked` failure
 * already carries the unlock hint in its message, so it is shown as is.
 */
export function actionFailedMessage(action: string, error: unknown): string {
  const message = error instanceof Error ? error.message : String(error)
  if (errorKind(error) === "connectionLocked") return message
  return `${action} failed: ${message}`
}
