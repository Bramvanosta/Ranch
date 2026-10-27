/**
 * Workers tab: table, detail, and the three ways of having
 * nothing to show (none attached / provider forbids CLIENT LIST / error).
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, within } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listWorkers: vi.fn() },
}))

import { commands, type QueueWorker, type QueueWorkers } from "@/lib/tauri/bindings"

import { WorkersTable } from "./WorkersTable"

const mocked = commands as unknown as { listWorkers: ReturnType<typeof vi.fn> }

afterEach(() => {
  vi.resetAllMocks()
})

function worker(overrides: Partial<QueueWorker> = {}): QueueWorker {
  return {
    id: "12",
    name: null,
    addr: "172.18.0.5:51234",
    ageSecs: 3_720,
    idleSecs: 2,
    lastCommand: "bzpopmin",
    library: "ioredis 5.4.1",
    raw: { id: "12", addr: "172.18.0.5:51234", name: "bull:ZW1haWxz", flags: "b", user: "" },
    ...overrides,
  }
}

function ok(data: Partial<QueueWorkers>) {
  return {
    status: "ok",
    data: { supported: true, unsupportedReason: null, workers: [], ...data },
  }
}

function renderTable() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  return render(<WorkersTable connectionId="c1" queueRef={{ prefix: "bull", name: "emails" }} />, {
    wrapper: Wrapper,
  })
}

describe("WorkersTable", () => {
  it("lists the workers with address, age, idle time and last command", async () => {
    mocked.listWorkers.mockResolvedValue(
      ok({
        workers: [
          worker(),
          worker({ id: "13", name: "mailer-eu", ageSecs: 45, idleSecs: 600, lastCommand: null }),
        ],
      }),
    )
    renderTable()
    const rows = await screen.findAllByTestId("workers-row")
    expect(rows).toHaveLength(2)
    expect(mocked.listWorkers).toHaveBeenCalledWith({
      connectionId: "c1",
      queueRef: { prefix: "bull", name: "emails" },
    })
    expect(screen.getByTestId("workers-count")).toHaveTextContent("2 workers attached")

    expect(within(rows[0]).getByText("unnamed")).toBeInTheDocument()
    expect(within(rows[0]).getByText("#12")).toBeInTheDocument()
    expect(within(rows[0]).getByText("172.18.0.5:51234")).toBeInTheDocument()
    expect(within(rows[0]).getByText("1h")).toBeInTheDocument()
    expect(within(rows[0]).getByText("bzpopmin")).toBeInTheDocument()

    expect(within(rows[1]).getByText("mailer-eu")).toBeInTheDocument()
    // Ten minutes without a command: flagged, with the reason on hover.
    expect(within(rows[1]).getByText("10m")).toHaveAttribute(
      "title",
      expect.stringMatching(/long job, or hung/),
    )
  })

  it("opens the raw client metadata of a worker", async () => {
    mocked.listWorkers.mockResolvedValue(ok({ workers: [worker()] }))
    renderTable()
    fireEvent.click(await screen.findByTestId("workers-row"))
    const detail = await screen.findByTestId("worker-detail")
    expect(within(detail).getByText("Unnamed worker")).toBeInTheDocument()
    expect(within(detail).getByText("bull:ZW1haWxz")).toBeInTheDocument()
    expect(within(detail).getByText("flags")).toBeInTheDocument()
    expect(within(detail).getByText(/ioredis 5.4.1/)).toBeInTheDocument()
  })

  it("explains what makes a worker appear when none is attached", async () => {
    mocked.listWorkers.mockResolvedValue(ok({ workers: [] }))
    renderTable()
    const empty = await screen.findByTestId("workers-empty")
    expect(empty).toHaveTextContent("Nothing is consuming this queue")
    expect(empty).toHaveTextContent("bull:emails")
    expect(screen.getByTestId("workers-count")).toHaveTextContent("No worker attached")
  })

  it("explains a provider that forbids CLIENT LIST — not an error, not 'zero workers'", async () => {
    mocked.listWorkers.mockResolvedValue(
      ok({
        supported: false,
        unsupportedReason: "NOPERM this user has no permissions to run the 'client|list' command",
      }),
    )
    renderTable()
    const state = await screen.findByTestId("workers-unsupported")
    expect(state).toHaveTextContent("does not allow listing clients")
    expect(state).toHaveTextContent(/NOPERM/)
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
    expect(screen.queryByTestId("workers-empty")).not.toBeInTheDocument()
  })

  it("shows a request failure as an error", async () => {
    mocked.listWorkers.mockResolvedValue({
      status: "error",
      error: { kind: "redis", message: "Connection refused", docUrl: null },
    })
    renderTable()
    expect(await screen.findByRole("alert", {}, { timeout: 4000 })).toHaveTextContent(
      /Connection refused/,
    )
  })
})
