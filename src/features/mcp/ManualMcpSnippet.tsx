import { CopyIcon } from "lucide-react"
/**
 * Manual MCP config block. Code block + Copy button.
 */
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import type { McpManualSnippet } from "@/lib/tauri/bindings"

export interface ManualMcpSnippetProps {
  snippet: McpManualSnippet
}

export function ManualMcpSnippet({ snippet }: ManualMcpSnippetProps) {
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(snippet.jsonSnippet)
      toast.success("Copied MCP config to clipboard", { duration: 2_000 })
    } catch (err) {
      toast.error(`Could not copy: ${err instanceof Error ? err.message : String(err)}`)
    }
  }
  return (
    <div
      className="flex flex-col gap-2 rounded-md border bg-(--surf)/20 p-4"
      data-testid="manual-mcp-snippet"
    >
      <header className="flex items-center justify-between gap-3">
        <h3 className="text-base font-semibold">Manual configuration</h3>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          onClick={onCopy}
          data-testid="manual-mcp-snippet-copy"
        >
          <CopyIcon className="mr-1.5 h-3.5 w-3.5" />
          Copy
        </Button>
      </header>
      <p className="text-xs text-(--text2)">
        For MCP clients Ranch can&apos;t auto-detect (custom integrations, future clients, CLI
        tools), paste this JSON into the client&apos;s MCP server config:
      </p>
      <pre className="overflow-x-auto rounded bg-(--bg) px-3 py-2 font-mono text-xs">
        <code>{snippet.jsonSnippet}</code>
      </pre>
      <p className="text-xs text-(--text2)">
        Binary path: <span className="font-mono break-all">{snippet.binaryPath}</span>
      </p>
    </div>
  )
}
