import type { RanchErrorDto } from "@/lib/tauri/bindings"

/**
 * What every generated command resolves to. The backend never rejects the
 * promise for a domain failure: it answers `{ status: "error", error }`.
 */
export type CommandResult<T> = { status: "ok"; data: T } | { status: "error"; error: RanchErrorDto }

/**
 * A backend failure as a thrown value. It keeps the `kind` discriminator
 * (`connectionLocked`, `jobNotFound`, `schemaNotRecognized`…) so retry
 * predicates, toasts and empty states can branch on it, and the help link
 * the backend attached, if any.
 */
export class RanchError extends Error {
  readonly kind: string
  readonly docUrl: string | null

  constructor({ kind, message, docUrl }: RanchErrorDto) {
    super(message)
    this.name = "RanchError"
    this.kind = kind
    this.docUrl = docUrl
  }
}

/** The data of an `ok` result; a `RanchError` for an `error` one. */
export function unwrap<T>(result: CommandResult<T>): T {
  if (result.status === "error") throw new RanchError(result.error)
  return result.data
}

/** The data of an `ok` result; `fallback` for an `error` one. */
export function okOr<T, F>(result: CommandResult<T>, fallback: F): T | F {
  return result.status === "ok" ? result.data : fallback
}

/**
 * The `kind` of a thrown value, when it is a backend failure. Anything else
 * (a network error, a bug) has no kind.
 */
export function errorKind(error: unknown): string | undefined {
  if (error instanceof RanchError) return error.kind
  if (error && typeof error === "object" && "kind" in error) {
    const { kind } = error as { kind: unknown }
    return typeof kind === "string" ? kind : undefined
  }
  return undefined
}
