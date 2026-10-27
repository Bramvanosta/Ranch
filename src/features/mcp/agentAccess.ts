import type { ApprovalMode } from "@/lib/tauri/bindings"

/**
 * A connection's agent access: Read-only (its flag), else Ask
 * or Free depending on the global approval mode. Behaviour is unchanged:
 * the per-connection control is still the read-only flag.
 */
export function accessOf(readonly: boolean, mode: ApprovalMode): "Read-only" | "Ask" | "Free" {
  if (readonly) return "Read-only"
  return mode === "off" ? "Free" : "Ask"
}
