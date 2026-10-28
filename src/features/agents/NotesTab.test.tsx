import { fireEvent, screen, waitFor } from "@testing-library/react"
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

import { useAiStore } from "@/lib/ai/aiFeatures"
import { type CauseNote, commands } from "@/lib/tauri/bindings"
import { renderWithProviders } from "@/test/renderWithProviders"
import { ok } from "@/test/results"

import { NotesTab } from "./NotesTab"

const mocked = vi.mocked(commands)

const note: CauseNote = {
  id: "n1",
  connectionId: "c1",
  connectionName: "prod-redis",
  queueId: "bull:emails",
  queueName: "emails",
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
  useAiStore.getState().setEnabled(true)
})

describe("Agents › Notes", () => {
  it("lists the notes with their cause, and the dismissed ones on request", async () => {
    mocked.listCauseNotes.mockImplementation(async ({ includeDismissed }) =>
      ok(includeDismissed ? [note, { ...note, id: "n2", dismissedAt: note.createdAt }] : [note]),
    )
    renderWithProviders(<NotesTab />)
    expect(await screen.findAllByTestId("agents-note")).toHaveLength(1)
    expect(screen.getByText("connect ECONNREFUSED <addr>")).toBeInTheDocument()
    expect(screen.getByText(/emails · prod-redis/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole("switch", { name: "Show dismissed notes" }))
    await waitFor(() => expect(screen.getAllByTestId("agents-note")).toHaveLength(2))
    expect(screen.getByText(/· dismissed/)).toBeInTheDocument()
  })

  it("reads nothing with AI features off", () => {
    useAiStore.getState().setEnabled(false)
    renderWithProviders(<NotesTab />)
    expect(screen.getByText("No notes yet")).toBeInTheDocument()
    expect(mocked.listCauseNotes).not.toHaveBeenCalled()
  })
})
