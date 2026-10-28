import { createFileRoute, useNavigate } from "@tanstack/react-router"
import { useEffect, useState } from "react"
import { z } from "zod"
import { type Need, useCapabilities } from "@/capabilities"
import { useCommandHandler } from "@/commands/handlers"
import { EnvTag } from "@/components/data/Env"
import { InfoStrip } from "@/components/data/InfoStrip"
import { Toolbar } from "@/components/data/Toolbar"
import { ViewPanel, ViewTabs, ViewTabsRoot } from "@/components/data/ViewTabs"
import { useConnections } from "@/data/connections/queries"
import { useQueues, useQueueWorkerCounts } from "@/data/queues/queries"
import { errorKind } from "@/data/result"
import { endpointLine } from "@/domain/bullmqEndpoint"
import { countOf } from "@/domain/stateCounts"
import { AskAiMenu, useAskAi, useAskAiDefaultAgent, useCodeFolder } from "@/features/askai"
import { ConnectionUnreachable } from "@/features/connections"
import { FailedJobsView } from "@/features/failed"
import { ConnectionFlows, FlowInspector } from "@/features/flows"
import { useConnectionEvents } from "@/features/live"
import { MapView } from "@/features/map"
import { ConnectionOverview, DEFAULT_METRICS_RANGE, METRICS_RANGE_VALUES } from "@/features/metrics"
import { ServerInfoItems } from "@/features/overview"
import { QueuesTable } from "@/features/queues"
import { LockButton } from "@/features/safety"
import { rowId, SchedulerInspectorFor, SchedulersView } from "@/features/schedulers"
import { RefreshControl } from "@/features/settings"
import { TimelineView } from "@/features/timeline"
import { ConnectionWorkers, WorkerInspector } from "@/features/workers"
import { Screen } from "@/layouts/Screen"
import { useAiEnabled } from "@/lib/ai/aiFeatures"

/**
 * Index route for `/connections/$connectionId`.
 *
 * One tab per `?view=` (`TAB_VIEWS`): the overview (the default, a
 * dashboard over every queue of the connection), the timeline, the map, the
 * queue table, schedulers, workers, flows and failed jobs. A tab the
 * connection's backend cannot serve is absent (`TAB_NEEDS`).
 *
 * URL is the source of truth: Zod validates `view` and
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

/**
 * What the backend must declare for a tab to be there. A link to a tab it
 * cannot serve opens the overview.
 */
const TAB_NEEDS: Record<TabView, Need | null> = {
  overview: null,
  timeline: "metrics.timeline",
  map: "server.topology",
  queues: null,
  schedulers: "schedules.list",
  workers: "workers.list",
  flows: "flows",
  failed: null,
}

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
  const capabilities = useCapabilities(connectionId)
  const hasTab = (view: TabView) => {
    const need = TAB_NEEDS[view]
    return need === null || capabilities.can(need)
  }
  // Until the backend is known, the URL's tab stands.
  const activeView: TabView =
    capabilities.descriptor === undefined || hasTab(search.view) ? search.view : "overview"
  // The Failed tab's badge: sum of the live queue counters
  // (already polled every second for the sidebar / grid; no extra call).
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
      const kind = errorKind(queuesQuery.error)
      setFailure({ connectionId, message: queuesQuery.error.message, kind })
    }
  }, [connectionId, queuesQuery.data, queuesQuery.error])
  const unreachable =
    failure !== null &&
    failure.connectionId === connectionId &&
    !queues &&
    failure.kind !== "schemaNotRecognized" &&
    failure.kind !== "connectionNotFound"
  const failedTotal = (queues ?? []).reduce((sum, q) => sum + countOf(q.counts, "failed"), 0)

  // The header shows the connection's human-readable name, not its UUID;
  // share the `["connections"]` query key with
  // `ConnectionSidebar` so we don't double-fetch.
  const { data: connections } = useConnections()
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
  const workerCounts = useQueueWorkerCounts(
    connectionId,
    queues,
    Boolean(queues) && capabilities.can("workers.count"),
  )
  const workerTotal = workerCounts
    ? [...workerCounts.values()].reduce((a, b) => a + b, 0)
    : undefined
  const where = connection ? endpointLine(connection) : null
  const infoStrip =
    connection && where ? (
      <InfoStrip label="Connection">
        {queues ? (
          <ServerInfoItems connection={connection} workers={workerTotal} />
        ) : (
          <span>{where}</span>
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
              { value: "overview" as const, label: "Overview" },
              // Every queue on one time axis.
              { value: "timeline" as const, label: "Timeline" },
              // Queues, workers and flows as a system.
              { value: "map" as const, label: "Map" },
              { value: "queues" as const, label: "Queues" },
              { value: "schedulers" as const, label: "Schedulers" },
              { value: "workers" as const, label: "Workers" },
              { value: "flows" as const, label: "Flows" },
              {
                value: "failed" as const,
                label: "Failed",
                testId: "connection-tab-failed",
                count: failedTotal > 0 ? (failedTotal > 999 ? "999+" : failedTotal) : undefined,
                tone: "danger" as const,
              },
            ].filter((tab) => hasTab(tab.value))}
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
