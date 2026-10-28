import { CircleCheckIcon, CopyIcon } from "lucide-react"
/**
 * `McpInstallerCard` renders one MCP-client
 * installer (Claude Desktop, Cursor, …) with its status badge,
 * Install button, and a Show-manual-instructions link.
 */
import { Button } from "@/components/ui/button"
import { useInstallMcpClient } from "@/data/mcp/mutations"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import type { McpClient, McpInstallerStatus } from "@/lib/tauri/bindings"

export interface McpInstallerCardProps {
  status: McpInstallerStatus
  onShowManual: () => void
  /**
   * Which client this card represents. Drives the display name and
   * the install handler.
   */
  client: McpClient
}

const DISPLAY_NAME: Record<McpClient, string> = {
  claudeDesktop: "Claude Desktop",
  claudeCode: "Claude Code",
  cursor: "Cursor",
  windsurf: "Windsurf",
  codex: "Codex",
}

/** The order the Setup tab and the wizard show the clients in. */
export const MCP_CLIENTS: McpClient[] = [
  "claudeDesktop",
  "claudeCode",
  "cursor",
  "windsurf",
  "codex",
]

export function McpInstallerCard({ status, onShowManual, client }: McpInstallerCardProps) {
  // The status reads again once installed (the hook's invalidation).
  const install = useInstallMcpClient()
  const copy = useCopyToClipboard()
  const busy = install.isPending
  const onInstall = () => install.mutate({ client, label: DISPLAY_NAME[client] })

  return (
    <div
      className="flex flex-col gap-3 rounded-md border bg-(--surf) p-4"
      data-testid={`mcp-installer-card-${client}`}
    >
      <header className="flex items-center justify-between gap-3">
        <h3 className="text-base font-semibold">{DISPLAY_NAME[client]}</h3>
        {status.installed ? (
          <span
            className="inline-flex items-center gap-1.5 rounded-md bg-(--state-completed)/15 px-2 py-0.5 text-xs text-(--state-completed-text)"
            data-testid={`mcp-installer-installed-badge-${client}`}
          >
            <CircleCheckIcon className="h-3.5 w-3.5" />
            Installed
          </span>
        ) : (
          <span className="rounded-md bg-(--surf) px-2 py-0.5 text-xs text-(--text2)">
            Not installed
          </span>
        )}
      </header>
      <p className="text-xs text-(--text2) wrap-anywhere">
        {status.kind === "clientCli" ? "Registered through its CLI · " : ""}Config:{" "}
        <span className="font-mono">{status.configPath || "<unresolved>"}</span>
      </p>
      {/* CLI clients: say when the CLI is missing, always offer the command. */}
      {status.kind === "clientCli" && status.cliFound === false ? (
        <p
          className="rounded-md border border-(--line) bg-(--surf)/40 px-3 py-2 text-xs text-(--text2)"
          data-testid={`mcp-installer-cli-missing-${client}`}
        >
          {DISPLAY_NAME[client]}&apos;s command-line tool was not detected. Install it, or run the
          command below yourself.
        </p>
      ) : null}
      {status.manualCommand ? (
        <div className="flex items-start gap-2">
          <code
            className="min-w-0 flex-1 wrap-anywhere rounded-md bg-(--surf) px-2 py-1.5 font-mono text-2xs"
            data-testid={`mcp-installer-command-${client}`}
          >
            {status.manualCommand}
          </code>
          <Button
            type="button"
            size="sm"
            variant="default"
            className="h-7 shrink-0 text-xs"
            onClick={() => void copy(status.manualCommand ?? "", "command")}
          >
            Copy
          </Button>
        </div>
      ) : null}
      {status.error ? (
        <p className="rounded-md border border-(--danger)/40 bg-(--danger)/10 px-3 py-2 text-xs text-(--state-failed-text)">
          {status.error}
        </p>
      ) : null}
      {status.installed ? (
        <p className="text-xs text-(--text2)">
          Restart {DISPLAY_NAME[client]} to activate Ranch&apos;s MCP server.
        </p>
      ) : null}
      <div className="flex items-center gap-2">
        <Button
          variant="primary"
          type="button"
          size="sm"
          onClick={onInstall}
          disabled={busy || (status.kind === "clientCli" && status.cliFound === false)}
          data-testid={`mcp-installer-install-button-${client}`}
        >
          {status.installed ? "Re-install" : "Install"}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          onClick={onShowManual}
          data-testid={`mcp-installer-manual-link-${client}`}
        >
          <CopyIcon className="mr-1.5 h-3.5 w-3.5" />
          Show manual instructions
        </Button>
      </div>
    </div>
  )
}
