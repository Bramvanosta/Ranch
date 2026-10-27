import { useQuery } from "@tanstack/react-query"
import { useState } from "react"

import { ManualMcpSnippet } from "@/features/mcp/ManualMcpSnippet"
import { McpSetupCards } from "@/features/mcp/McpSetupCards"
import { commands } from "@/lib/tauri/bindings"

/**
 * one-click installers for the agent clients on this
 * machine, and the manual snippet (shown on request, or when no client is
 * detected).
 */
export function AgentsSetup() {
  const [showManual, setShowManual] = useState(false)
  const statusQuery = useQuery({
    queryKey: ["mcpInstallerStatus"],
    queryFn: async () => {
      const r = await commands.getMcpInstallerStatus()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
  })
  const snippetQuery = useQuery({
    queryKey: ["mcpManualSnippet"],
    queryFn: async () => {
      const r = await commands.getMcpManualSnippet()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
  })

  return (
    <div className="flex flex-col gap-4" data-testid="settings-mcp-installers">
      {statusQuery.isPending ? (
        <p className="text-sm text-(--text2)">Reading which agent clients are installed</p>
      ) : statusQuery.isError ? (
        <p className="rounded-md border border-(--danger)/40 bg-(--danger)/10 px-3 py-2 text-sm text-(--state-failed-text)">
          Could not read installer status: {statusQuery.error.message}
        </p>
      ) : (
        <McpSetupCards
          overview={statusQuery.data}
          onChange={() => statusQuery.refetch()}
          onShowManual={() => setShowManual(true)}
        />
      )}
      {showManual && snippetQuery.data ? <ManualMcpSnippet snippet={snippetQuery.data} /> : null}
    </div>
  )
}
