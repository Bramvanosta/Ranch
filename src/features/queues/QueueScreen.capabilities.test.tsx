import { QueryClientProvider } from "@tanstack/react-query"
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router"
import { act, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

/**
 * The queue screen follows what the connection's backend declares. A kind
 * that keeps a state of its own ("parked") and has no schedulers and no
 * queue control shows its state as a tab, and no Schedulers tab, Pause
 * button or palette entry; BullMQ shows everything. (Its pill:
 * `src/capabilities/capabilities.test.tsx`.)
 */

const answers: Record<string, (args: never) => unknown> = {
  getKvSetting: () => null,
  getConnectionLockState: () => "unlocked",
  listQueues: () => [
    {
      id: "q:emails",
      name: "emails",
      isPaused: false,
      counts: { waiting: 1, failed: 0, parked: 2 },
    },
  ],
  listJobs: (args: { args: { state: string } }) => ({
    jobs: [],
    total: 0,
    state: args.args.state,
    hasMore: false,
    scanned: null,
    scanCapped: false,
  }),
  listCronJobs: () => [],
  listWorkers: () => ({ supported: true, workers: [] }),
  listGroups: () => [],
}

// Every command answers: those above with their data, the others with an error.
vi.mock("@/lib/tauri/bindings", () => ({
  commands: new Proxy(
    {},
    {
      get: (_, name: string) =>
        vi.fn(async (...args: unknown[]) =>
          name in answers
            ? { status: "ok", data: answers[name]({ args: args[0] } as never) }
            : { status: "error", error: { kind: "unsupported", message: name, docUrl: null } },
        ),
    },
  ),
}))

import { BULLMQ_CAPABILITIES, BULLMQ_REDIS_INFO } from "@/backend/fake/backends"
import { connectionKeys } from "@/data/connections/keys"
import { CommandPalette } from "@/features/palette"
import type { BackendInfo } from "@/lib/tauri/bindings"
import { Route as QueueRoute } from "@/routes/connections/$connectionId/queues/$queueId/index"
import { useCommandPaletteStore } from "@/stores/useCommandPaletteStore"
import { aConnection } from "@/test/factories"
import { testQueryClient } from "@/test/renderWithProviders"

const PARKING_INFO: BackendInfo = {
  ...BULLMQ_REDIS_INFO,
  kind: "parking",
  capabilities: {
    ...BULLMQ_CAPABILITIES,
    jobStates: [
      { code: "waiting", label: "Waiting", tone: "pending", common: true },
      { code: "failed", label: "Failed", tone: "danger", common: true },
      { code: "parked", label: "Parked", tone: "warning", common: false },
    ],
    schedules: { list: false, write: false, pause: false },
    workers: { list: false, count: false },
    queueControl: false,
  },
}

function renderQueue(backend: string, search = "") {
  const client = testQueryClient([BULLMQ_REDIS_INFO, PARKING_INFO])
  client.setQueryData(connectionKeys.all, [aConnection({ id: "c1", backend })])
  const root = createRootRoute({
    component: () => (
      <>
        <Outlet />
        <CommandPalette />
      </>
    ),
  })
  const queue = createRoute({
    getParentRoute: () => root,
    path: "/connections/$connectionId/queues/$queueId/",
    component: QueueRoute.options.component,
    validateSearch: QueueRoute.options.validateSearch,
  })
  const router = createRouter({
    routeTree: root.addChildren([queue]),
    history: createMemoryHistory({ initialEntries: [`/connections/c1/queues/q:emails/${search}`] }),
  })
  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  )
}

const tab = (name: string) => screen.queryByRole("tab", { name: new RegExp(`^${name}`) })

async function paletteItems() {
  act(() => useCommandPaletteStore.getState().setOpen(true))
  const palette = await screen.findByTestId("command-palette")
  return within(palette)
}

afterEach(() => {
  act(() => useCommandPaletteStore.getState().setOpen(false))
})

describe("the queue screen of a backend without schedulers, with a state of its own", () => {
  it("has no Schedulers or Workers tab, no Pause, and a Parked tab", async () => {
    renderQueue("parking")
    expect(await screen.findByTestId("job-state-tab-parked")).toHaveTextContent("Parked")
    expect(screen.getByTestId("job-state-tab-waiting")).toBeInTheDocument()
    // The headline states it does not keep have no tab.
    expect(screen.queryByTestId("job-state-tab-delayed")).not.toBeInTheDocument()
    expect(tab("Schedulers")).not.toBeInTheDocument()
    expect(tab("Workers")).not.toBeInTheDocument()
    expect(tab("Flows")).toBeInTheDocument()
    expect(screen.queryByTestId("queue-pause-toggle")).not.toBeInTheDocument()
    expect(screen.getByTestId("queue-add-job")).toBeInTheDocument()
  })

  it("opens the jobs from a link to the Schedulers tab", async () => {
    renderQueue("parking", "?view=cron")
    expect(await screen.findByTestId("job-state-tab-parked")).toHaveAttribute(
      "aria-selected",
      "false",
    )
    await waitFor(() => expect(screen.queryByTestId("cron-jobs-empty")).not.toBeInTheDocument())
    expect(screen.queryByTestId("scheduler-new")).not.toBeInTheDocument()
  })

  it("offers no Pause in the palette", async () => {
    renderQueue("parking")
    await screen.findByTestId("job-state-tab-parked")
    const palette = await paletteItems()
    expect(palette.queryByText(/Pause/)).not.toBeInTheDocument()
    expect(palette.getByText(/Add Job/)).toBeInTheDocument()
  })
})

describe("the queue screen of BullMQ", () => {
  it("has every tab and action, and its own states only when they hold jobs", async () => {
    renderQueue("bullmq-redis")
    expect(await screen.findByTestId("queue-pause-toggle")).toBeInTheDocument()
    for (const name of ["Schedulers", "Workers", "Flows", "Metrics"]) {
      expect(tab(name)).toBeInTheDocument()
    }
    for (const state of ["waiting", "active", "delayed", "failed", "completed"]) {
      expect(screen.getByTestId(`job-state-tab-${state}`)).toBeInTheDocument()
    }
    expect(screen.queryByTestId("job-state-tab-prioritized")).not.toBeInTheDocument()
    // "parked" is not a BullMQ state: its count is no tab.
    expect(screen.queryByTestId("job-state-tab-parked")).not.toBeInTheDocument()
    const palette = await paletteItems()
    expect(palette.getByText(/Pause “emails”/)).toBeInTheDocument()
  })
})
