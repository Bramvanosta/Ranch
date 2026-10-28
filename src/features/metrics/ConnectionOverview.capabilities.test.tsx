import { screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

/**
 * The connection's cards follow what its backend declares. A kind with no
 * events, server info, throughput or queue metrics shows the tiles it can
 * count and Needs attention, and asks for nothing else: no live events, no
 * busiest queues, no metrics (nor their error), no server figures. BullMQ
 * shows them all. (The scheduler table: `features/queues/CronJobsView.test.tsx`.)
 */

const answers: Record<string, () => unknown> = {
  getKvSetting: () => null,
  listQueues: () => [{ id: "q:emails", name: "emails", isPaused: false, counts: { waiting: 2 } }],
  listFailureCauses: () => ({ causes: [], scanned: 0, capped: false }),
  countQueueWorkers: () => ({ supported: true, counts: [] }),
  getMcpSafetySettings: () => ({ readonlyDefault: false, approvalMode: "always" }),
}

const asked: string[] = []

// Every command answers: those above with their data, the others with an error.
vi.mock("@/lib/tauri/bindings", () => ({
  commands: new Proxy(
    {},
    {
      get: (_, name: string) =>
        vi.fn(async () => {
          asked.push(name)
          return name in answers
            ? { status: "ok", data: answers[name]() }
            : { status: "error", error: { kind: "unsupported", message: name, docUrl: null } }
        }),
    },
  ),
}))
// The live events panel has its own tests (features/overview).
vi.mock("@/features/overview/LiveEvents", () => ({
  LiveEvents: () => <div data-testid="live-events-stub" />,
}))
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: React.ReactNode }) => <a href="/">{children}</a>,
}))
vi.mock("recharts", () => {
  const Passthrough = ({ children }: { children?: React.ReactNode }) => <div>{children}</div>
  const Leaf = () => null
  return {
    ResponsiveContainer: Passthrough,
    BarChart: Passthrough,
    LineChart: Passthrough,
    CartesianGrid: Leaf,
    XAxis: Leaf,
    YAxis: Leaf,
    Bar: Leaf,
    Line: Leaf,
    Tooltip: Leaf,
    Legend: Leaf,
  }
})

import { BULLMQ_CAPABILITIES, BULLMQ_REDIS_INFO } from "@/backend/fake/backends"
import { connectionKeys } from "@/data/connections/keys"
import { ServerInfoItems } from "@/features/overview"
import type { BackendInfo } from "@/lib/tauri/bindings"
import { aConnection } from "@/test/factories"
import { renderWithProviders, testQueryClient } from "@/test/renderWithProviders"

import { ConnectionOverview } from "./ConnectionOverview"

const QUIET_INFO: BackendInfo = {
  ...BULLMQ_REDIS_INFO,
  kind: "quiet",
  capabilities: {
    ...BULLMQ_CAPABILITIES,
    metrics: { queue: false, throughput: false, timeline: false, native: false },
    events: { read: false, follow: false },
    server: { info: false, topology: false },
  },
}

function clientFor(backend: string) {
  const client = testQueryClient([BULLMQ_REDIS_INFO, QUIET_INFO])
  client.setQueryData(connectionKeys.all, [aConnection({ id: "c1", backend })])
  return client
}

function renderCards(backend: string) {
  const connection = aConnection({ id: "c1", backend })
  return renderWithProviders(
    <>
      <ServerInfoItems connection={connection} workers={undefined} />
      <ConnectionOverview connectionId="c1" range="24h" onRangeChange={vi.fn()} />
    </>,
    { client: clientFor(backend) },
  )
}

afterEach(() => {
  asked.length = 0
})

describe("the connection's cards", () => {
  it("are absent, and not asked for, when the backend does not declare their read", async () => {
    renderCards("quiet")
    expect(await screen.findByTestId("needs-attention")).toBeInTheDocument()
    await waitFor(() => expect(asked).toContain("listQueues"))
    expect(screen.getByTestId("overview-tile-waiting")).toBeInTheDocument()

    expect(screen.queryByTestId("live-events-stub")).not.toBeInTheDocument()
    expect(screen.queryByTestId("overview-tile-completed")).not.toBeInTheDocument()
    expect(screen.queryByTestId("busiest-queues")).not.toBeInTheDocument()
    expect(screen.queryByTestId("connection-overview-error")).not.toBeInTheDocument()
    expect(screen.queryByTestId("connection-overview-loading")).not.toBeInTheDocument()

    for (const command of ["getConnectionMetrics", "getThroughput", "getServerInfo"]) {
      expect(asked).not.toContain(command)
    }
  })

  it("are all there for BullMQ", async () => {
    renderCards("bullmq-redis")
    expect(await screen.findByTestId("live-events-stub")).toBeInTheDocument()
    expect(screen.getByTestId("overview-tile-completed")).toBeInTheDocument()
    expect(screen.getByTestId("busiest-queues")).toBeInTheDocument()
    await waitFor(() => {
      for (const command of ["getConnectionMetrics", "getThroughput", "getServerInfo"]) {
        expect(asked).toContain(command)
      }
    })
  })
})
