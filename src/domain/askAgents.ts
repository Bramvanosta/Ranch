/** Who Ask AI hands things to by default, and where code lives. */
export type AskAgent = "claudeCode" | "claudeDesktop" | "cursor" | "copy"

export const ASK_AGENTS: Array<{ value: AskAgent; label: string }> = [
  { value: "claudeCode", label: "Claude Code" },
  { value: "claudeDesktop", label: "Claude Desktop" },
  { value: "cursor", label: "Cursor" },
  { value: "copy", label: "Copy prompt" },
]

export function isAskAgent(value: unknown): value is AskAgent {
  return ASK_AGENTS.some((a) => a.value === value)
}
