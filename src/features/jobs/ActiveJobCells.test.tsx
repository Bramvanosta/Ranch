import { QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { testQueryClient } from "@/test/renderWithProviders"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { getQueueMetrics: vi.fn() },
}))

import { commands, type Job } from "@/lib/tauri/bindings"

import { ActiveProgressCell, ActiveRunningCell } from "./ActiveJobCells"
import { progressKey, STALL_MS, useProgressWatchStore } from "./progressWatch"

const mocked = commands as unknown as { getQueueMetrics: ReturnType<typeof vi.fn> }
const queue = "bull:emails"
const now = 10_000_000

function job(overrides: Partial<Job> = {}): Job {
  return {
    jobRef: { queueId: queue, queueName: "emails", id: "7" },
    name: "send",
    state: "active",
    data: "{}",
    opts: "{}",
    attemptsMade: 0,
    timestamp: now - 600_000,
    processedOn: now - 300_000,
    finishedOn: null,
    delay: null,
    failedReason: null,
    stacktrace: [],
    returnValue: null,
    parentKey: null,
    progress: '{"percentage":40,"message":"page 4 of 10"}',
    processedBy: "worker-1",
    lockTtlMs: 20_000,
    ...overrides,
  }
}

function wrap(children: ReactNode) {
  const client = testQueryClient()
  return render(<QueryClientProvider client={client}>{children}</QueryClientProvider>)
}

beforeEach(() => {
  useProgressWatchStore.setState({ seen: new Map() })
  mocked.getQueueMetrics.mockResolvedValue({
    status: "ok",
    data: {
      usualDurations: [
        {
          queueId: "bull:emails",
          queueName: "emails",
          name: "send",
          medianMs: 120_000,
          samples: 9,
        },
      ],
    },
  })
})
afterEach(() => vi.resetAllMocks())

describe("Active tab cells", () => {
  it("progress shows its bar, value and note", () => {
    wrap(<ActiveProgressCell job={job()} connectionId="c1" now={now} />)
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "40")
    expect(screen.getByTestId("job-progress-note")).toHaveTextContent("page 4 of 10")
  })

  it("no progress at all reads as a dash", () => {
    wrap(<ActiveProgressCell job={job({ progress: null })} connectionId="c1" now={now} />)
    expect(screen.getByTestId("job-progress-cell")).toHaveTextContent("-")
  })

  it("unchanged for 3 min with a renewed lock: amber, and says so", () => {
    const value = '{"percentage":40,"message":"page 4 of 10"}'
    useProgressWatchStore.setState({
      seen: new Map([[progressKey("c1", "bull:emails", "7"), { value, since: now - STALL_MS }]]),
    })
    wrap(<ActiveProgressCell job={job({ progress: value })} connectionId="c1" now={now} />)
    expect(screen.getByTestId("job-progress-cell")).toHaveAttribute("data-stalled", "true")
    expect(screen.getByTestId("job-progress-stalled")).toHaveTextContent("no progress for 3 min")
  })

  it("running time against the usual time of its type", async () => {
    wrap(<ActiveRunningCell job={job()} connectionId="c1" now={now} />)
    expect(await screen.findByTestId("job-running-usual")).toHaveTextContent("usual 2.0 min")
    // 5 min running, usually 2: amber.
    await waitFor(() =>
      expect(screen.getByTestId("job-running-time")).toHaveAttribute("data-slow", "true"),
    )
  })
})
