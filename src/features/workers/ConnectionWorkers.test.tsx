import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listQueues: vi.fn(), listWorkers: vi.fn(), listRecentJobs: vi.fn() },
}))
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="#x">{children}</a>,
}))

import { commands, type Queue, type QueueWorker } from "@/lib/tauri/bindings"

import { ConnectionWorkers, WorkerInspector } from "./ConnectionWorkers"
import { useLastWorkerSeen } from "./workerState"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const queue = (name: string, waiting: number, isPaused = false): Queue =>
  ({
    id: `bull:${name}`,
    name,
    counts: {
      waiting,
      active: 0,
      completed: 0,
      failed: 0,
      delayed: 0,
      prioritized: 0,
      paused: 0,
      waitingChildren: 0,
      stalled: 0,
    },
    isPaused,
  }) as unknown as Queue

const worker = (id: string, name: string, cmd: string, library: string): QueueWorker => ({
  id,
  name,
  addr: `10.0.0.${id}:5000`,
  ageSecs: 3600,
  idleSecs: 2,
  lastCommand: cmd,
  library,
  raw: { id, name: `bull:x:w:${name}`, cmd },
})

function wrap(children: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{children}</QueryClientProvider>)
}

beforeEach(() => {
  useLastWorkerSeen.setState({ seen: new Map() })
  mocked.listQueues.mockResolvedValue({
    status: "ok",
    data: [queue("emails", 3), queue("reports", 5), queue("idle-q", 0)],
  })
  mocked.listWorkers.mockImplementation(async ({ queueId }: { queueId: string }) => ({
    status: "ok",
    data: {
      supported: true,
      unsupportedReason: null,
      workers:
        queueId === "bull:emails"
          ? [
              worker("1", "mailer-1", "bzpopmin", "ioredis 5.4.1"),
              worker("2", "mailer-2", "evalsha", "ioredis 5.3.2"),
            ]
          : [],
    },
  }))
})
afterEach(() => vi.resetAllMocks())

describe("ConnectionWorkers", () => {
  it("tiles, workers grouped by queue, their state, the older library highlighted", async () => {
    wrap(<ConnectionWorkers connectionId="c1" selected={null} onSelect={vi.fn()} />)
    await waitFor(() => expect(screen.getAllByTestId("worker-row")).toHaveLength(2))
    expect(screen.getByTestId("workers-tile-connected")).toHaveTextContent("2")
    expect(screen.getByTestId("workers-tile-working")).toHaveTextContent("1")
    expect(screen.getByTestId("workers-tile-without")).toHaveTextContent("2")
    expect(screen.getByTestId("workers-tile-libraries")).toHaveTextContent("older: ioredis 5.3.2")
    const rows = screen.getAllByTestId("worker-row")
    expect(rows[0]).toHaveTextContent("Idle")
    expect(rows[1]).toHaveTextContent("Working")
    const libs = screen.getAllByTestId("worker-library")
    expect(libs[1]).toHaveAttribute("data-older", "true")
    expect(libs[0]).not.toHaveAttribute("data-older")
  })

  it("a queue with waiting jobs and no worker gets a banner", async () => {
    wrap(<ConnectionWorkers connectionId="c1" selected={null} onSelect={vi.fn()} />)
    const banner = await screen.findByTestId("workers-orphan")
    expect(banner).toHaveTextContent("reports has 5 jobs waiting and no worker")
    expect(banner).toHaveTextContent("no worker seen since Ranch opened")
    // An idle queue without jobs is not a problem.
    expect(screen.getAllByTestId("workers-orphan")).toHaveLength(1)
  })

  it("says so when the Redis refuses CLIENT LIST", async () => {
    mocked.listWorkers.mockResolvedValue({
      status: "ok",
      data: { supported: false, unsupportedReason: "unknown command 'CLIENT'", workers: [] },
    })
    wrap(<ConnectionWorkers connectionId="c1" selected={null} onSelect={vi.fn()} />)
    expect(await screen.findByTestId("workers-unsupported")).toHaveTextContent("unknown command")
  })

  it("a row opens the worker; the inspector lists the jobs it processed and its client fields", async () => {
    const onSelect = vi.fn()
    wrap(<ConnectionWorkers connectionId="c1" selected={null} onSelect={onSelect} />)
    fireEvent.click((await screen.findAllByTestId("worker-row"))[0] as HTMLElement)
    expect(onSelect).toHaveBeenCalledWith("bull:emails/1")

    mocked.listRecentJobs.mockResolvedValue({
      status: "ok",
      data: {
        unreadableQueues: [],
        jobs: [
          {
            jobRef: { queueId: "bull:emails", queueName: "emails", id: "9" },
            name: "send",
            state: "completed",
            processedBy: "mailer-1",
            finishedOn: Date.now(),
            timestamp: 0,
            processedOn: null,
          },
          {
            jobRef: { queueId: "bull:emails", queueName: "emails", id: "8" },
            name: "send",
            state: "completed",
            processedBy: "mailer-2",
            finishedOn: Date.now(),
            timestamp: 0,
            processedOn: null,
          },
        ],
      },
    })
    wrap(<WorkerInspector connectionId="c1" id="bull:emails/1" onClose={vi.fn()} />)
    const processed = await screen.findByTestId("worker-processed")
    expect(processed.querySelectorAll("li")).toHaveLength(1)
    expect(processed).toHaveTextContent("send #9")
    expect(screen.getByTestId("worker-raw")).toHaveTextContent("bull:x:w:mailer-1")
  })
})
