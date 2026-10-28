import type { McpInstallerOverview, McpSafetySettings } from "@/lib/tauri/bindings"
import type { FakeHandlers } from "./handlers"
import { jobsHandlers } from "./jobs"
import { demoNote, previewDismissed } from "./seed"
import { type Args, HOUR, MIN } from "./types"
import { a, connectionOf, world } from "./world"

/** Agents: MCP safety, approvals, activity, notes, Ask AI and the installers. */
export const agentsHandlers = {
  // MCP
  get_mcp_safety_settings: () => world.safety,
  set_mcp_safety_settings: ({ settings }) => {
    world.safety = settings as McpSafetySettings
    return world.safety
  },
  set_connection_mcp_readonly: (args) => {
    connectionOf(a(args).connectionId).mcpReadonly = (a(args).readonly as boolean | null) ?? null
    return null
  },
  list_pending_approvals: () => world.approvals,
  // Ask AI: the preview masks like the default policy and
  // pretends to launch (nothing to start in a browser).
  share_for_agents: ({ json }) =>
    String(json).replace(
      /"([^"]*(?:email|phone|token|password|secret|iban)[^"]*|card[^"]*)":\s*("(?:[^"\\]|\\.)*"|[\d.]+)/gi,
      '"$1":"•••"',
    ),
  launch_claude_code: () => null,
  open_agent_app: () => null,
  // agents' notes: one on the biggest cause of c-prod/emails.
  list_cause_notes: (args) => {
    const input = a(args)
    const conn = world.connections.find((c) => c.id === "c-prod")
    if (!conn) return []
    if (input.connectionId && input.connectionId !== conn.id) return []
    const causes = jobsHandlers.list_failure_causes({
      args: { connectionId: conn.id, queue: "emails" },
    }) as { causes: Array<{ signature: string }> } | undefined
    const signature = causes?.causes[0]?.signature
    if (signature === undefined || previewDismissed.has("note-1")) {
      return input.includeDismissed && signature !== undefined
        ? [demoNote(conn, signature, true)]
        : []
    }
    return [demoNote(conn, signature, false)]
  },
  dismiss_cause_note: ({ noteId }) => {
    previewDismissed.add(String(noteId))
    return null
  },
  // Agents › Approvals, earlier decisions.
  list_approval_history: () => {
    const conn = world.connections.find((c) => c.id === "c-prod") ?? world.connections[0]
    if (!conn) return []
    const at = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString()
    return [
      ["h-1", 12, "consumed", "claude-ai", "bulk_retry_jobs", null],
      ["h-2", 95, "denied", "cursor", "drain_queue", "Not on production, ask me first"],
      ["h-3", 300, "expired", "claude-ai", "remove_job", null],
      ["h-4", 1440, "consumed", "claude-code", "retry_job", null],
    ].map(([id, minutes, status, client, tool, reason]) => ({
      id: String(id),
      createdAt: at(Number(minutes)),
      decidedAt: at(Number(minutes) - 1),
      status: String(status),
      clientHint: String(client),
      tool: String(tool),
      reason: reason === null ? null : String(reason),
      connectionId: conn.id,
      connectionName: conn.name,
    }))
  },
  set_ai_features: ({ enabled }) => {
    world.kv.set("ai_enabled", enabled ? "true" : "false")
    if (!enabled) world.approvals = []
    return Boolean(enabled)
  },
  decide_pending_approval: (args) => {
    world.approvals = world.approvals.filter((p) => p.id !== a(args).id)
    return "decided"
  },
  list_mcp_activity: (args) => {
    const f = (a(args).filter ?? {}) as Args
    const events = world.activity.filter(
      (e) =>
        (!f.kind || e.kind === f.kind) &&
        (!f.status || e.status === f.status) &&
        (!f.sessionId || e.sessionId === f.sessionId),
    )
    return { events, total: events.length }
  },
  list_mcp_activity_sessions: () => [
    {
      sessionId: "s-now",
      clientHint: "Claude Desktop",
      startedAt: new Date(Date.now() - 8 * MIN).toISOString(),
      lastAt: new Date(Date.now() - 2 * MIN).toISOString(),
      total: 5,
      writes: 3,
      notSuccessful: 2,
    },
    {
      sessionId: "s-morning",
      clientHint: "Cursor",
      startedAt: new Date(Date.now() - 5 * HOUR - 3 * MIN).toISOString(),
      lastAt: new Date(Date.now() - 5 * HOUR).toISOString(),
      total: 3,
      writes: 1,
      notSuccessful: 1,
    },
  ],
  export_mcp_activity: () => JSON.stringify(world.activity, null, 2),
  clear_mcp_activity: () => {
    const n = world.activity.length
    world.activity = []
    return n
  },
  get_mcp_installer_status: () => world.installers,
  install_mcp_for_client: ({ client }) => install(String(client)),
  get_mcp_manual_snippet: () => ({
    binaryPath: "/Applications/Ranch.app/Contents/MacOS/ranch-mcp",
    jsonSnippet: JSON.stringify(
      { mcpServers: { ranch: { command: "/Applications/Ranch.app/Contents/MacOS/ranch-mcp" } } },
      null,
      2,
    ),
  }),
} satisfies FakeHandlers

function install(client: string) {
  const status = world.installers[client as keyof McpInstallerOverview]
  if (status.cliFound === false) {
    status.error = "The client's command-line tool was not found"
    return status
  }
  status.installed = true
  return status
}
