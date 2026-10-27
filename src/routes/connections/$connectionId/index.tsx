import { useQuery } from "@tanstack/react-query"
import { createFileRoute, useNavigate } from "@tanstack/react-router"
import { useEffect, useState } from "react"
import { z } from "zod"
import { useCommandHandler } from "@/commands/handlers"
import { EnvTag } from "@/components/data/Env"
import { InfoStrip } from "@/components/data/InfoStrip"
import { Toolbar } from "@/components/data/Toolbar"
import { ViewPanel, ViewTabs, ViewTabsRoot } from "@/components/data/ViewTabs"
import { AskAiMenu } from "@/features/askai/AskAiMenu"
import { useAskAiDefaultAgent, useCodeFolder } from "@/features/askai/askAiSettings"
import { useAskAi } from "@/features/askai/useAskAi"
import { ConnectionUnreachable } from "@/features/connections/ConnectionUnreachable"
import { useQueueWorkerCounts } from "@/features/connections/useSidebarQueues"
import { FailedJobsView } from "@/features/failed/FailedJobsView"
import { ConnectionFlows, FlowInspector } from "@/features/flows/ConnectionFlows"
import { useConnectionEvents } from "@/features/live/useConnectionEvents"
import { MapView } from "@/features/map/MapView"
import { ConnectionOverview } from "@/features/metrics/ConnectionOverview"
import { DEFAULT_METRICS_RANGE, METRICS_RANGE_VALUES } from "@/features/metrics/ranges"
import { ServerInfoItems } from "@/features/overview/ServerInfo"
import { QueuesTable } from "@/features/queues/QueuesTable"
import { useQueues } from "@/features/queues/useQueues"
import { LockButton } from "@/features/safety/LockButton"
import { SchedulerInspectorFor } from "@/features/schedulers/SchedulerInspector"
import { SchedulersView } from "@/features/schedulers/SchedulersView"
import { rowId } from "@/features/schedulers/useConnectionSchedulers"
import { RefreshControl } from "@/features/settings/RefreshControl"
import { TimelineView } from "@/features/timeline/TimelineView"
import { ConnectionWorkers, WorkerInspector } from "@/features/workers/ConnectionWorkers"
import { Screen } from "@/layouts/Screen"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { commands } from "@/lib/tauri/bindings"

/**
 * Index route for `/connections/$connectionId`.
 *
 * Two tabs:
 *   - `?view=overview` (default) — env-level dashboard, weighted-average
 *     aggregate across every queue on the connection (matches the
 *     BullStudio reference UI).
 *   - `?view=queues` — the responsive queue grid.
 *
 * URL is the source of truth — Zod validates `view` and
 * falls back to `overview`. The parent route hosts the `<Outlet />` so
 * navigating into a specific queue still re-mounts that subtree.
 *
 * The parent route is
 * layout-only so child routes (queue detail, job detail) mount
 * cleanly via the `<Outlet />`.
 */
const TAB_VIEWS = [
  "overview",
  "timeline",
  "map",
  "queues",
  "schedulers",
  "workers",
  "flows",
  "failed",
] as const
type TabView = (typeof TAB_VIEWS)[number]

const searchSchema = z.object({
  view: z.enum(TAB_VIEWS).default("overview").catch("overview"),
  // Metrics window of the Overview tab.
  range: z.enum(METRICS_RANGE_VALUES).default(DEFAULT_METRICS_RANGE).catch(DEFAULT_METRICS_RANGE),
  // The Failed tab: filters + page.
  fqueue: z.string().default("").catch(""),
  fname: z.string().default("").catch(""),
  freason: z.string().default("").catch(""),
  fpage: z.number().int().min(0).default(0).catch(0),
  // The scheduler open in the inspector (`queue/key`).
  scheduler: z.string().optional().catch(undefined),
  // The worker open in the inspector (`queue/clientId`).
  worker: z.string().optional().catch(undefined),
  // The flow open in the inspector (`queue/jobId`).
  flow: z.string().optional().catch(undefined),
  // The Map shows every queue, idle ones included.
  all: z.coerce.number().optional().catch(undefined),
})

export const Route = createFileRoute("/connections/$connectionId/")({
  component: ConnectionIndexRoute,
  validateSearch: searchSchema,
})

function ConnectionIndexRoute() {
  const { connectionId } = Route.useParams()
  const search = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  const activeView: TabView = search.view
  // The Failed tab's badge: sum of the live queue counters
  // (already polled every second for the sidebar / grid — no extra call).
  // Follow this connection's event streams while it is on screen.
  useConnectionEvents(connectionId)
  const queuesQuery = useQueues(connectionId)
  const queues = queuesQuery.data
  // Nothing loaded and the list failed: Redis cannot be reached. One screen
  // says so for the whole connection, instead of one red line per tab. (A
  // Redis that answers but holds no BullMQ keys is not this: the tabs explain.)
  //
  // The failure is remembered until data arrives: a query with no data goes
  // back to `pending` on every poll, so reading `isError` alone made the page
  // flip between "Loading…" and the error once a second.
  const [failure, setFailure] = useState<{
    connectionId: string
    message: string
    kind?: string
  } | null>(null)
  useEffect(() => {
    if (queuesQuery.data) {
      setFailure(null)
    } else if (queuesQuery.error) {
      const kind = (queuesQuery.error as Error & { kind?: string }).kind
      setFailure({ connectionId, message: queuesQuery.error.message, kind })
    }
  }, [connectionId, queuesQuery.data, queuesQuery.error])
  const unreachable =
    failure !== null &&
    failure.connectionId === connectionId &&
    !queues &&
    failure.kind !== "schemaNotRecognized" &&
    failure.kind !== "connectionNotFound"
  const failedTotal = (queues ?? []).reduce((sum, q) => sum + Number(q.counters?.failed ?? 0), 0)

  // The header shows the connection's human-readable name, not its UUID;
  // share the `["connections"]` query key with
  // `ConnectionSidebar` so we don't double-fetch.
  const { data: connections } = useQuery({
    queryKey: ["connections"],
    queryFn: async () => {
      const result = await commands.listConnections()
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
  })
  const connection = connections?.find((c) => c.id === connectionId)
  const title = connection?.name ?? "Connection"

  // Ask AI about this connection when nothing narrower is open.
  const askAi = useAskAi()
  const defaultAgent = useAskAiDefaultAgent()
  const folder = useCodeFolder(connection?.groupId)
  const aiOn = useAiEnabled()
  useCommandHandler(
    "agents.askAi",
    () => connection && void askAi({ kind: "connection", connection }, defaultAgent, folder),
    { enabled: aiOn && Boolean(connection), priority: 1, label: "Ask AI About This Connection" },
  )

  const toolbar = (
    <Toolbar
      title={title}
      titleTestId="connection-overview-title"
      meta={
        connection ? (
          <EnvTag
            color={connection.envColor}
            label={connection.envLabel}
            data-testid="connection-env-badge"
          />
        ) : null
      }
      actions={
        <>
          {connection ? (
            <AskAiMenu target={{ kind: "connection", connection }} testId="connection-ask-ai" />
          ) : null}
          <LockButton connectionId={connectionId} />
          <RefreshControl connectionId={connectionId} />
        </>
      }
    />
  )
  // The server and its workers, once Redis answers.
  const workerCounts = useQueueWorkerCounts(connectionId, queues, Boolean(queues))
  const workerTotal = workerCounts
    ? [...workerCounts.values()].reduce((a, b) => a + b, 0)
    : undefined
  const infoStrip = connection?.host ? (
    <InfoStrip label="Connection">
      {queues ? (
        <ServerInfoItems connection={connection} workers={workerTotal} />
      ) : (
        <span title={`${connection.host}:${connection.port} (db ${connection.db})`}>
          {connection.host}:{connection.port} · db {connection.db}
        </span>
      )}
    </InfoStrip>
  ) : null

  if (unreachable) {
    return (
      <Screen toolbar={toolbar} infoStrip={infoStrip}>
        <ConnectionUnreachable
          connection={connection}
          message={failure.message}
          kind={failure.kind}
          onRetry={() => void queuesQuery.refetch()}
          retrying={queuesQuery.isFetching}
        />
      </Screen>
    )
  }

  return (
    <ViewTabsRoot
      value={activeView}
      onValueChange={(v) => {
        navigate({ search: (prev) => ({ ...prev, view: v as TabView }) })
      }}
      className="h-full gap-0"
    >
      <Screen
        toolbar={toolbar}
        infoStrip={infoStrip}
        inspector={
          activeView === "schedulers" && search.scheduler ? (
            <SchedulerInspectorFor
              connectionId={connectionId}
              id={search.scheduler}
              onClose={() => navigate({ search: (prev) => ({ ...prev, scheduler: undefined }) })}
            />
          ) : activeView === "flows" && search.flow ? (
            <FlowInspector
              connectionId={connectionId}
              id={search.flow}
              onClose={() => navigate({ search: (prev) => ({ ...prev, flow: undefined }) })}
            />
          ) : activeView === "workers" && search.worker ? (
            <WorkerInspector
              connectionId={connectionId}
              id={search.worker}
              onClose={() => navigate({ search: (prev) => ({ ...prev, worker: undefined }) })}
            />
          ) : undefined
        }
        tabs={
          <ViewTabs
            items={[
              { value: "overview", label: "Overview" },
              // Every queue on one time axis.
              { value: "timeline", label: "Timeline" },
              // Queues, workers and flows as a system.
              { value: "map", label: "Map" },
              { value: "queues", label: "Queues" },
              { value: "schedulers", label: "Schedulers" },
              { value: "workers", label: "Workers" },
              { value: "flows", label: "Flows" },
              {
                value: "failed",
                label: "Failed",
                testId: "connection-tab-failed",
                count: failedTotal > 0 ? (failedTotal > 999 ? "999+" : failedTotal) : undefined,
                tone: "danger",
              },
            ]}
          />
        }
      >
        <ViewPanel value="overview" className="min-h-0 flex-1 overflow-auto pr-2">
          <ConnectionOverview
            connectionId={connectionId}
            range={search.range}
            onRangeChange={(range) => navigate({ search: (prev) => ({ ...prev, range }) })}
          />
        </ViewPanel>
        <ViewPanel value="timeline" className="min-h-0 flex-1 overflow-auto pr-2">
          <TimelineView connectionId={connectionId} />
        </ViewPanel>
        <ViewPanel value="map" className="flex min-h-0 flex-1 flex-col">
          <MapView
            connectionId={connectionId}
            onlyActive={search.all !== 1}
            onOnlyActiveChange={(only) =>
              navigate({ search: (prev) => ({ ...prev, all: only ? undefined : 1 }) })
            }
          />
        </ViewPanel>
        <ViewPanel value="queues" className="flex min-h-0 flex-1 flex-col">
          <QueuesTable connectionId={connectionId} />
        </ViewPanel>
        <ViewPanel value="schedulers" className="flex min-h-0 flex-1 flex-col overflow-auto pr-2">
          <SchedulersView
            connectionId={connectionId}
            selected={search.scheduler ?? null}
            onSelect={(row) => navigate({ search: (prev) => ({ ...prev, scheduler: rowId(row) }) })}
          />
        </ViewPanel>
        <ViewPanel value="workers" className="min-h-0 flex-1 overflow-auto pr-2">
          <ConnectionWorkers
            connectionId={connectionId}
            selected={search.worker ?? null}
            onSelect={(worker) => navigate({ search: (prev) => ({ ...prev, worker }) })}
          />
        </ViewPanel>
        <ViewPanel value="flows" className="min-h-0 flex-1 overflow-auto pr-2">
          <ConnectionFlows
            connectionId={connectionId}
            selected={search.flow ?? null}
            onSelect={(flow) => navigate({ search: (prev) => ({ ...prev, flow }) })}
          />
        </ViewPanel>
        <ViewPanel value="failed" className="min-h-0 flex-1 overflow-auto pr-2">
          <FailedJobsView
            connectionId={connectionId}
            filter={{ queue: search.fqueue, name: search.fname, reason: search.freason }}
            page={search.fpage}
            onFilterChange={(f) =>
              navigate({
                search: (prev) => ({
                  ...prev,
                  fqueue: f.queue,
                  fname: f.name,
                  freason: f.reason,
                  fpage: 0,
                }),
              })
            }
            onPageChange={(fpage) => navigate({ search: (prev) => ({ ...prev, fpage }) })}
          />
        </ViewPanel>
      </Screen>
    </ViewTabsRoot>
  )
}
