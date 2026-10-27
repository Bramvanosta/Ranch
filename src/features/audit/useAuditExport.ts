import { useMutation } from "@tanstack/react-query"
import { toast } from "sonner"

import { type AuditAction, type AuditSource, commands } from "@/lib/tauri/bindings"

/**
 * Mutation that fetches the export payload from the
 * backend, then triggers a browser-side download via the standard
 * `<a download>` pattern (no Tauri filesystem write — the backend
 * stays read-only / safe).
 *
 * Surfaces a sonner toast on success / failure. The success toast
 * mentions the chosen filename so the user knows where to find it.
 */
export interface AuditExportArgs {
  from?: string
  to?: string
  connectionId?: string
  action?: AuditAction
  source?: AuditSource
  format: "json" | "csv"
}

export function useAuditExport() {
  return useMutation({
    mutationFn: async (args: AuditExportArgs) => {
      const result = await commands.exportAuditLog({
        from: args.from ?? null,
        to: args.to ?? null,
        connectionId: args.connectionId ?? null,
        action: args.action ?? null,
        source: args.source ?? null,
        format: args.format,
      })
      if (result.status === "error") {
        throw new Error(result.error.message)
      }
      return result.data
    },
    onSuccess: ({ content, suggestedFilename }) => {
      // Create a Blob + anchor click — the standard "save a file"
      // pattern that works in Tauri's webview without an extra
      // filesystem-write capability.
      const blob = new Blob([content], {
        type: suggestedFilename.endsWith(".json") ? "application/json" : "text/csv",
      })
      const url = URL.createObjectURL(blob)
      const a = document.createElement("a")
      a.href = url
      a.download = suggestedFilename
      document.body.appendChild(a)
      a.click()
      document.body.removeChild(a)
      URL.revokeObjectURL(url)
      toast.success(`Exported ${suggestedFilename}`, { duration: 3_000 })
    },
    onError: (err) => {
      toast.error(`Export failed: ${err instanceof Error ? err.message : "unknown error"}`, {
        duration: 4_000,
      })
    },
  })
}
