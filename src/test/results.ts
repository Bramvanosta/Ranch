import type { RanchErrorDto } from "@/lib/tauri/bindings"

/**
 * What a generated command resolves to, for `vi.mocked(commands.x)`:
 *
 *   vi.mocked(commands.listJobs).mockResolvedValue(ok(output))
 *   vi.mocked(commands.retryJob).mockResolvedValue(err("connectionLocked"))
 */

export function ok<T>(data: T) {
  return { status: "ok" as const, data }
}

export function err(kind: string, message = `${kind} (test)`) {
  const error: RanchErrorDto = { kind, message, docUrl: null }
  return { status: "error" as const, error }
}
