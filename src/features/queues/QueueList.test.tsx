import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor } from "@testing-library/react"
import type { ComponentProps, ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listQueues: vi.fn() },
}))

// Stub TanStack Router's `Link` so we don't need a router context just to
// test the queue rows. The mock renders a plain anchor with the resolved
// path so tests can still assert hrefs if needed.
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    to,
    params,
    children,
    ...rest
  }: {
    to: string
    params?: Record<string, string>
    children: ReactNode
  } & ComponentProps<"a">) => {
    const href = Object.entries(params ?? {}).reduce(
      (acc, [k, v]) => acc.replace(`$${k}`, encodeURIComponent(v)),
      to,
    )
    return (
      <a href={href} {...rest}>
        {children}
      </a>
    )
  },
}))

import { commands } from "@/lib/tauri/bindings"

import { QueueList } from "./QueueList"

const mocked = commands as unknown as { listQueues: ReturnType<typeof vi.fn> }

afterEach(() => {
  vi.resetAllMocks()
})

function renderList(connectionId = "conn-x") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <QueueList connectionId={connectionId} />
    </QueryClientProvider>,
  )
}

describe("QueueList", () => {
  it("shows skeleton placeholders while loading", () => {
    // Don't resolve the mock; keeps the query in `isLoading`.
    mocked.listQueues.mockReturnValue(new Promise(() => {}))
    renderList()
    expect(screen.getByLabelText(/Loading queues/i)).toBeInTheDocument()
  })

  it("shows the empty state when no queues are found", async () => {
    mocked.listQueues.mockResolvedValue({ status: "ok", data: [] })
    renderList()
    await waitFor(() => expect(screen.getByText(/No BullMQ queues found/i)).toBeInTheDocument())
  })

  it("renders the error state with the verbatim error message", async () => {
    mocked.listQueues.mockResolvedValue({
      status: "error",
      error: {
        kind: "schemaNotRecognized",
        message: "BullMQ schema not recognized (detected version: 7.0.0)",
        docUrl: null,
      },
    })
    renderList()
    await waitFor(() => expect(screen.getByText(/Could not list queues/i)).toBeInTheDocument())
    expect(screen.getByText(/detected version: 7\.0\.0/i)).toBeInTheDocument()
  })

  it("renders the queue list with name + counters when populated", async () => {
    mocked.listQueues.mockResolvedValue({
      status: "ok",
      data: [
        {
          id: "bull:emails",
          name: "emails",
          counts: {
            waiting: 50,
            active: 0,
            completed: 1000,
            failed: 5,
            delayed: 10,
            prioritized: 0,
            paused: 0,
          },
          isPaused: false,
        },
        {
          id: "bull:images",
          name: "images",
          counts: {
            waiting: 22,
            active: 1,
            completed: 8,
            failed: 0,
            delayed: 0,
            prioritized: 0,
            paused: 0,
          },
          isPaused: true,
        },
      ],
    })
    renderList()
    await waitFor(() => expect(screen.getByText("emails")).toBeInTheDocument())
    expect(screen.getByText("images")).toBeInTheDocument()

    // Counters render as inert spans inside a Link (each
    // row is wrapped in a Link so clicking anywhere navigates into the queue).
    expect(screen.getByTitle("Failed: 5")).toBeInTheDocument()
    expect(screen.getByTitle("Completed: 8")).toBeInTheDocument()

    // Paused badge appears next to `images` only.
    const pausedBadges = screen.getAllByText(/^paused$/i)
    expect(pausedBadges).toHaveLength(1)

    // Each row is a link to its queue-detail route.
    expect(screen.getByTestId("queue-row-emails")).toHaveAttribute(
      "href",
      "/connections/conn-x/queues/bull%3Aemails",
    )
  })
})
