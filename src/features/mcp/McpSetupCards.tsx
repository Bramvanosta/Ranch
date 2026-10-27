import type { McpInstallerOverview } from "@/lib/tauri/bindings"

import { MCP_CLIENTS, McpInstallerCard } from "./McpInstallerCard"

interface McpSetupCardsProps {
  overview: McpInstallerOverview
  onChange: () => void
  onShowManual: () => void
  /** Wizard: one column. Settings: two. */
  columns?: 1 | 2
}

/**
 * One installer card per supported client, shared by
 * Agents › Setup & safety and the first-run wizard's MCP step.
 */
export function McpSetupCards({
  overview,
  onChange,
  onShowManual,
  columns = 2,
}: McpSetupCardsProps) {
  return (
    <div
      className={columns === 2 ? "grid grid-cols-1 gap-3 md:grid-cols-2" : "grid grid-cols-1 gap-3"}
    >
      {MCP_CLIENTS.map((client) => {
        // Tolerate a status the backend did not send rather than crash the page.
        const status = overview[client]
        return status ? (
          <McpInstallerCard
            key={client}
            client={client}
            status={status}
            onChange={onChange}
            onShowManual={onShowManual}
          />
        ) : null
      })}
    </div>
  )
}
