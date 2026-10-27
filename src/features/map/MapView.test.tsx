import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    getThroughput: vi.fn(),
    getTopology: vi.fn(),
    listFailureCauses: vi.fn(),
  },
}))
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/x">{children}</a>,
}))
const queues = vi.hoisted(() => ({ value: [] as unknown[] }))
vi.mock("@/features/queues/useQueues", () => ({ useQueues: () => ({ data: queues.value }) }))
const workerGroups = vi.hoisted(() => ({ value: [] as unknown[] }))
vi.mock("@/features/workers/ConnectionWorkers", () => ({
  useConnectionWorkers: () => ({ groups: workerGroups.value }),
}))

import { commands } from "@/lib/tauri/bindings"

import { MapView } from "./MapView"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const ok = <T,>(data: T) => ({ status: "ok", data })

const queue = (name: string, waiting = 0, paused = false) => ({
  queueRef: { prefix: "bull", name },
  isPaused: paused,
  counters: { waiting, active: 0, prioritized: 0, failed: 0, completed: 0, delayed: 0 },
})

afterEach(() => vi.resetAllMocks())

function wrap(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>)
}

function seed() {
  queues.value = [queue("emails", 4), queue("orders"), queue("idle"), queue("reports", 2)]
  workerGroups.value = [
    {
      queue: queues.value[0],
      workers: [{ worker: { id: "1", name: "mailer", addr: "10.0.0.1:1" } }],
    },
  ]
  mocked.getThroughput.mockResolvedValue(
    ok({
      range: "1h",
      bucketMs: 3_600_000,
      bucketStartsMs: [0],
      queues: [
        {
          queueRef: { prefix: "bull", name: "emails" },
          metricsOff: false,
          completed: [540],
          failed: [60],
        },
        {
          queueRef: { prefix: "bull", name: "orders" },
          metricsOff: false,
          completed: [60],
          failed: [0],
        },
      ],
      completed: [],
      failed: [],
      metricsOffQueues: 0,
    }),
  )
  mocked.getTopology.mockResolvedValue(
    ok({
      links: [
        {
          parent: { prefix: "bull", name: "orders" },
          child: { prefix: "bull", name: "emails" },
          jobs: 2,
        },
      ],
      queues: [
        {
          queueRef: { prefix: "bull", name: "emails" },
          addedPerMinute: 12,
          addedCapped: true,
          oldestWaitingMs: Date.now() - 120_000,
        },
      ],
    }),
  )
  mocked.listFailureCauses.mockResolvedValue(
    ok({
      causes: [{ signature: "SMTP <n> rejected" }],
      scanned: 60,
      scanCapped: false,
      totalFailed: 60,
    }),
  )
}

describe("MapView", () => {
  it("lays out active queues, pools and a No worker node", async () => {
    seed()
    wrap(<MapView connectionId="c1" onlyActive onOnlyActiveChange={vi.fn()} />)
    await waitFor(() => expect(screen.getAllByTestId("map-queue")).toHaveLength(3))
    expect(screen.getAllByTestId("map-queue").map((n) => n.textContent)).toEqual([
      expect.stringContaining("emails"),
      expect.stringContaining("orders"),
      expect.stringContaining("reports"),
    ])
    expect(screen.getByText("1 idle queue hidden")).toBeInTheDocument()
    expect(screen.getByTestId("map-pool")).toHaveTextContent("mailer")
    expect(screen.getByTestId("map-no-worker")).toBeInTheDocument()
    expect(screen.getByRole("list", { name: "Legend" })).toHaveTextContent("paused or idle")
  })

  it("a clicked queue shows in / out, failure, main cause and waiting", async () => {
    seed()
    wrap(<MapView connectionId="c1" onlyActive onOnlyActiveChange={vi.fn()} />)
    const emails = await screen.findByRole("button", { name: /emails/ })
    await waitFor(() => expect(emails).toHaveTextContent("10/min"))
    fireEvent.click(emails)
    const details = screen.getByTestId("map-details")
    expect(details).toHaveTextContent("≥ 12 / 10 per minute")
    expect(details).toHaveTextContent("10.0%")
    expect(await within(details).findByText("SMTP <n> rejected")).toBeInTheDocument()
    expect(details).toHaveTextContent("4, oldest 2m ago")
    expect(within(details).getByText("Open queue")).toBeInTheDocument()
    expect(within(details).getByText("Show workers")).toBeInTheDocument()
  })

  it("the activity switch asks for every queue", async () => {
    seed()
    const onChange = vi.fn()
    wrap(<MapView connectionId="c1" onlyActive onOnlyActiveChange={onChange} />)
    await screen.findAllByTestId("map-queue")
    fireEvent.click(screen.getByRole("switch"))
    expect(onChange).toHaveBeenCalledWith(false)
  })
})
