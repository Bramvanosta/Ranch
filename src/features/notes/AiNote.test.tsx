import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listCauseNotes: vi.fn(),
    dismissCauseNote: vi.fn(),
    listConnections: vi.fn(),
    getKvSetting: vi.fn(),
  },
}))
vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn(), message: vi.fn() }),
}))
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }))
const copied = vi.hoisted(() => [] as string[])
vi.mock("@/hooks/useCopyToClipboard", () => ({
  useCopyToClipboard: () => async (text: string) => {
    copied.push(text)
    return true
  },
}))

import { useAiStore } from "@/lib/ai/aiFeatures"
import { type CauseNote, commands } from "@/lib/tauri/bindings"

import { NotesTab } from "../agents/NotesTab"
import { AiNote } from "./AiNote"
import { followUpPrompt, noteForCause } from "./useCauseNotes"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const ok = <T,>(data: T) => ({ status: "ok", data })

const note: CauseNote = {
  id: "n1",
  connectionId: "c1",
  connectionName: "prod-redis",
  queueRef: { prefix: "bull", name: "emails" },
  signature: "connect ECONNREFUSED <addr>",
  text: "The SMTP relay is down.",
  steps: ["Restart it", "Retry the jobs"],
  clientHint: "claude-code",
  createdAt: new Date(Date.now() - 5 * 60_000).toISOString(),
  jobsRead: ["41"],
  dismissedAt: null,
}

afterEach(() => {
  vi.resetAllMocks()
  copied.length = 0
  useAiStore.getState().setEnabled(true)
})

function wrap(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>)
}

describe("AiNote", () => {
  it("shows the text, the steps and who wrote it, with its actions", async () => {
    mocked.dismissCauseNote.mockResolvedValue(ok(null))
    const onRetry = vi.fn()
    wrap(<AiNote note={note} retryCount={12} onRetry={onRetry} />)
    expect(screen.getByText("The SMTP relay is down.")).toBeInTheDocument()
    expect(screen.getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "Restart it",
      "Retry the jobs",
    ])
    expect(screen.getByTestId("ai-note-meta")).toHaveTextContent(
      "Note from Claude Code · via Ranch MCP · 5m ago",
    )
    fireEvent.click(screen.getByRole("button", { name: "Retry 12" }))
    expect(onRetry).toHaveBeenCalled()
    fireEvent.click(screen.getByRole("button", { name: "Copy" }))
    await waitFor(() =>
      expect(copied).toContain("The SMTP relay is down.\n\n1. Restart it\n2. Retry the jobs"),
    )
    // The follow-up goes through the Ask AI menu.
    fireEvent.pointerDown(screen.getByTestId("ai-note-follow-up"), {
      button: 0,
      pointerType: "mouse",
    })
    fireEvent.click(await screen.findByTestId("ask-ai-copy"))
    await waitFor(() => expect(copied[copied.length - 1]).toContain("annotate_cause"))
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }))
    await waitFor(() => expect(mocked.dismissCauseNote).toHaveBeenCalledWith("n1"))
  })

  it("matches a note to its cause by signature and queue", () => {
    const cause = { signature: note.signature, queueNames: ["emails", "images"] }
    expect(noteForCause([note], cause)).toBe(note)
    expect(noteForCause([note], cause, "images")).toBeUndefined()
    expect(noteForCause([note], { ...cause, signature: "other" })).toBeUndefined()
    expect(followUpPrompt(note)).toContain('"connect ECONNREFUSED <addr>" of queue emails')
  })
})

describe("Agents › Notes", () => {
  it("lists the notes with their cause, and the dismissed ones on request", async () => {
    mocked.listCauseNotes.mockImplementation(async ({ includeDismissed }) =>
      ok(includeDismissed ? [note, { ...note, id: "n2", dismissedAt: note.createdAt }] : [note]),
    )
    wrap(<NotesTab />)
    expect(await screen.findAllByTestId("agents-note")).toHaveLength(1)
    expect(screen.getByText("connect ECONNREFUSED <addr>")).toBeInTheDocument()
    expect(screen.getByText(/emails · prod-redis/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole("switch", { name: "Show dismissed notes" }))
    await waitFor(() => expect(screen.getAllByTestId("agents-note")).toHaveLength(2))
    expect(screen.getByText(/· dismissed/)).toBeInTheDocument()
  })

  it("reads nothing with AI features off", () => {
    useAiStore.getState().setEnabled(false)
    wrap(<NotesTab />)
    expect(screen.getByText("No notes yet")).toBeInTheDocument()
    expect(mocked.listCauseNotes).not.toHaveBeenCalled()
  })
})
