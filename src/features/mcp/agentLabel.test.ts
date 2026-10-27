import { describe, expect, it } from "vitest"

import { agentIdentity, agentLabel } from "./agentLabel"

describe("agentLabel", () => {
  it("names the clients we recognise, whatever they append to the prefix", () => {
    expect(agentLabel("claude-code")).toBe("Claude Code")
    expect(agentLabel("Claude-Code/1.2.3")).toBe("Claude Code")
    expect(agentLabel("local-agent-mode-ranch")).toBe("Claude (agent mode)")
    expect(agentLabel("cursor-vscode")).toBe("Cursor")
    expect(agentLabel("mcp-inspector")).toBe("MCP Inspector")
  })

  it("humanises an identifier it does not know", () => {
    expect(agentLabel("my-own_agent")).toBe("My own agent")
  })

  it("has nothing to say about a missing client", () => {
    expect(agentLabel(null)).toBeNull()
    expect(agentLabel("   ")).toBeNull()
  })

  it("keeps the raw identifier beside the label", () => {
    expect(agentIdentity("claude-code", "stdio")).toBe("Claude Code · claude-code (stdio)")
    // Nothing gained by repeating a name that is already readable.
    expect(agentIdentity("Ranch", "stdio")).toBe("Ranch (stdio)")
    expect(agentIdentity(null)).toBeNull()
  })
})
