/**
 * `McpInstallerCard` UX states.
 */
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    installMcpForClient: vi.fn(),
  },
}))
const copySpy = vi.fn()
vi.mock("@/hooks/useCopyToClipboard", () => ({ useCopyToClipboard: () => copySpy }))

import { commands, type McpInstallerStatus } from "@/lib/tauri/bindings"

import { McpInstallerCard } from "./McpInstallerCard"

const mockedInstall = commands.installMcpForClient as unknown as ReturnType<typeof vi.fn>

afterEach(() => {
  vi.resetAllMocks()
})

function status(overrides: Partial<McpInstallerStatus> = {}): McpInstallerStatus {
  return {
    client: "claudeDesktop",
    configPath: "/Users/alex/Library/Application Support/Claude/claude_desktop_config.json",
    installed: false,
    configDirExists: true,
    error: null,
    kind: "jsonMerge",
    cliFound: null,
    manualCommand: null,
    ...overrides,
  }
}

describe("McpInstallerCard", () => {
  it("renders not-installed state with config path", () => {
    render(
      <McpInstallerCard
        client="claudeDesktop"
        status={status()}
        onChange={vi.fn()}
        onShowManual={vi.fn()}
      />,
    )
    expect(screen.getByText(/Claude Desktop/)).toBeInTheDocument()
    expect(screen.getByText(/Not installed/i)).toBeInTheDocument()
    expect(screen.getByText(/claude_desktop_config\.json/)).toBeInTheDocument()
  })

  it("renders installed state with the green badge + restart hint", () => {
    render(
      <McpInstallerCard
        client="claudeDesktop"
        status={status({ installed: true })}
        onChange={vi.fn()}
        onShowManual={vi.fn()}
      />,
    )
    expect(screen.getByTestId("mcp-installer-installed-badge-claudeDesktop")).toBeInTheDocument()
    expect(screen.getByText(/Restart Claude Desktop to activate/i)).toBeInTheDocument()
  })

  it("clicking Install calls the generic installer with the client and notifies onChange", async () => {
    mockedInstall.mockResolvedValue({
      status: "ok",
      data: status({ installed: true }),
    })
    const onChange = vi.fn()
    render(
      <McpInstallerCard
        client="claudeDesktop"
        status={status()}
        onChange={onChange}
        onShowManual={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByTestId("mcp-installer-install-button-claudeDesktop"))
    await waitFor(() => expect(mockedInstall).toHaveBeenCalledWith("claudeDesktop"))
    await waitFor(() => expect(onChange).toHaveBeenCalled())
  })

  // ── Clients registered through their own CLI ───────────────

  const claudeCode = (over: Partial<McpInstallerStatus> = {}) =>
    status({
      client: "claudeCode",
      configPath: "/Users/alex/.claude.json",
      kind: "clientCli",
      cliFound: true,
      manualCommand: "claude mcp add --scope user ranch /Applications/Ranch.app/ranch-mcp",
      ...over,
    })

  it("a CLI client shows its command and installs through the same generic command", async () => {
    mockedInstall.mockResolvedValue({ status: "ok", data: claudeCode({ installed: true }) })
    render(
      <McpInstallerCard
        client="claudeCode"
        status={claudeCode()}
        onChange={vi.fn()}
        onShowManual={vi.fn()}
      />,
    )
    expect(screen.getByText("Claude Code")).toBeInTheDocument()
    expect(screen.getByTestId("mcp-installer-command-claudeCode")).toHaveTextContent(
      "claude mcp add --scope user ranch",
    )
    fireEvent.click(screen.getByRole("button", { name: "Copy" }))
    expect(copySpy).toHaveBeenCalledWith(expect.stringContaining("claude mcp add"), "command")

    fireEvent.click(screen.getByTestId("mcp-installer-install-button-claudeCode"))
    await waitFor(() => expect(mockedInstall).toHaveBeenCalledWith("claudeCode"))
  })

  it("when the CLI is missing: says so, disables Install, still offers the command", () => {
    render(
      <McpInstallerCard
        client="claudeCode"
        status={claudeCode({ cliFound: false })}
        onChange={vi.fn()}
        onShowManual={vi.fn()}
      />,
    )
    expect(screen.getByTestId("mcp-installer-cli-missing-claudeCode")).toHaveTextContent(
      /was not detected/,
    )
    expect(screen.getByTestId("mcp-installer-install-button-claudeCode")).toBeDisabled()
    expect(screen.getByTestId("mcp-installer-command-claudeCode")).toBeInTheDocument()
  })

  it("clicking Show manual instructions calls onShowManual", () => {
    const onShowManual = vi.fn()
    render(
      <McpInstallerCard
        client="claudeDesktop"
        status={status()}
        onChange={vi.fn()}
        onShowManual={onShowManual}
      />,
    )
    fireEvent.click(screen.getByTestId("mcp-installer-manual-link-claudeDesktop"))
    expect(onShowManual).toHaveBeenCalled()
  })

  it("displays the error message inline when status.error is set", () => {
    render(
      <McpInstallerCard
        client="claudeDesktop"
        status={status({ error: "Permission denied writing config dir" })}
        onChange={vi.fn()}
        onShowManual={vi.fn()}
      />,
    )
    expect(screen.getByText(/Permission denied writing config dir/i)).toBeInTheDocument()
  })
})
