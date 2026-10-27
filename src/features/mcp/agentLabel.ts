/**
 * UI review 2026-09-21 — what to call the agent on the other end.
 *
 * All we know about it is the `clientInfo.name` it declared in the MCP
 * `initialize` handshake (`mcp_pending_approvals.client_hint`,
 * `audit_log_entries.agent_client`). Those identifiers are written for
 * machines — `claude-code`, `local-agent-mode-ranch`, `cursor-vscode` — and
 * read badly in a sentence a human has to decide on.
 *
 * So: a product name for the clients we recognise, a humanised identifier for
 * the ones we don't, and **the raw string stays on screen** next to it (an
 * "Agent" row, a tooltip). A label is a courtesy; the identifier is the
 * audit trail, and a client that lies about its name must not be able to
 * hide behind a friendly label.
 */

/** Matched as a prefix, longest first — `claude-code` before `claude`. */
const KNOWN_CLIENTS: ReadonlyArray<readonly [prefix: string, label: string]> = [
  ["claude-code", "Claude Code"],
  ["claude-desktop", "Claude Desktop"],
  ["claude-ai", "Claude"],
  ["local-agent-mode", "Claude (agent mode)"],
  ["cursor", "Cursor"],
  ["roo-cline", "Roo Code"],
  ["cline", "Cline"],
  ["windsurf", "Windsurf"],
  ["continue", "Continue"],
  ["zed", "Zed"],
  ["vscode", "VS Code"],
  ["mcp-inspector", "MCP Inspector"],
  ["openai-codex", "Codex"],
  ["codex", "Codex"],
  ["gemini-cli", "Gemini CLI"],
]

/**
 * A readable name for an MCP client identifier. `null` / blank → `null`,
 * so callers pick their own fallback ("An agent", "the agent").
 */
export function agentLabel(client: string | null | undefined): string | null {
  const raw = client?.trim()
  if (!raw) return null
  const lower = raw.toLowerCase()
  for (const [prefix, label] of KNOWN_CLIENTS) {
    // The prefix must end on a boundary: `claude-code`, `claude-code-cli`,
    // `claude-code/1.2.3` — but never `claude-codex`.
    if (lower === prefix) return label
    if (lower.startsWith(prefix) && /[^a-z0-9]/.test(lower.charAt(prefix.length))) {
      return label
    }
  }
  return humanise(raw)
}

/** `my-agent_v2` → `My agent v2`. Identifiers are lower-kebab by convention. */
function humanise(raw: string): string {
  const words = raw
    .split(/[-_.\s]+/)
    .filter(Boolean)
    .join(" ")
  if (!words) return raw
  return words.charAt(0).toUpperCase() + words.slice(1)
}

/** `Claude Code · claude-code (stdio)` — the full identity, for a details row. */
export function agentIdentity(
  client: string | null | undefined,
  transport?: string | null,
): string | null {
  const raw = client?.trim()
  if (!raw) return null
  const label = agentLabel(raw)
  const suffix = transport ? ` (${transport})` : ""
  return label && label !== raw ? `${label} · ${raw}${suffix}` : `${raw}${suffix}`
}
