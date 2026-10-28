import { createFileRoute, useNavigate } from "@tanstack/react-router"
import { useEffect, useState } from "react"
import { type Need, useCapabilities } from "@/capabilities"
import { useCommandHandler } from "@/commands/handlers"
import { Segmented } from "@/components/data/Segmented"
import { ViewPanel, ViewTabs, ViewTabsRoot } from "@/components/data/ViewTabs"
import { useConnections } from "@/data/connections/queries"
import { useConnectionLockState } from "@/data/safety/queries"
import { useCronJobs } from "@/data/schedulers/queries"
import { jobPageSize } from "@/data/settings/settings"
import { useSetting } from "@/data/settings/useSetting"
import { useWorkers } from "@/data/workers/queries"
import type { PageSize } from "@/domain/pageSize"
import { countOf } from "@/domain/stateCounts"
import { FlowsView } from "@/features/flows"
import {
  AddJobDialog,
  BulkActionBar,
  FailureCausesView,
  type JobFilterScope,
  JobInspector,
  JobsPanel,
  visibleStateTabs,
} from "@/features/jobs"
import { useConnectionEvents } from "@/features/live"
import { QueueOverview } from "@/features/metrics"
import {
  CronJobsView,
  jobsSearch as jobsSearchOf,
  type QueueSearch,
  type QueueTabView,
  QueueToolbar,
  queueSearchSchema,
  useRouteQueue,
} from "@/features/queues"
import { WorkersTable } from "@/features/workers"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import { Screen } from "@/layouts/Screen"
import type { Job, JobSort, JobState } from "@/lib/tauri/bindings"
import { openRouteOrSay, queueRoute } from "@/lib/windows/openRoute"
import { useJobSelectionStore } from "@/stores/useJobSelectionStore"
import { useShellStore } from "@/stores/useShellStore"

/**
 * Drill-into-queue route.
 *
 * Two top-level views via `view`:
 *   - `?view=jobs`: state chips + filter
 *     input + JobList. In queue scope `fname` (job id or
 *     name), `ferror` (failure reason), `sort`/`dir` and `size` are
 *     sent to `list_jobs` and run across the whole state; the scope
 *     toggle (`fscope=cross`) widens to all queues of the connection
 *     via `useCrossQueueJobs`, filtered client-side.
 *   - `?view=cron`: CronJobsView.
 *   - `?view=workers`: WorkersTable. The tab carries the live
 *     worker count, and the header warns when nothing consumes the queue.
 *
 * URL is the source of truth. The Zod schema validates every
 * param; bad values default to safe ones. The state tabs are the states
 * the connection's backend keeps, and a view it cannot serve has no tab:
 * a link to one, or to a state it does not keep, opens the jobs.
 */

/** What the backend must declare for each view to have a tab. */
const VIEW_NEEDS: Record<QueueTabView, Need | null> = {
  jobs: null,
  cron: "schedules.list",
  workers: "workers.list",
  flows: "flows",
  overview: "metrics.queue",
}

export const Route = createFileRoute("/connections/$connectionId/queues/$queueId/")({
  component: QueueDetailComponent,
  validateSearch: queueSearchSchema,
})

function QueueDetailComponent() {
  const { connectionId, queueId: param } = Route.useParams()
  const search = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  // Clear the job selection whenever the queue or
  // state-filter changes. Selection scope is "this queue + this
  // state-filter": switching either erases the selection so the
  // BulkActionBar doesn't carry stale entries across navigation.
  const clearSelection = useJobSelectionStore((s) => s.clear)
  // biome-ignore lint/correctness/useExhaustiveDependencies: the queue + search.state are the intentional triggers (the effect fires when either route param changes)
  useEffect(() => {
    clearSelection()
  }, [param, search.state, clearSelection])

  // Follow this connection's event streams while it is on screen.
  useConnectionEvents(connectionId)
  const { queue, queueId, queueName, moveTo } = useRouteQueue(connectionId, param)
  useEffect(() => {
    if (moveTo)
      navigate({ params: { connectionId, queueId: moveTo }, search: (s) => s, replace: true })
  }, [moveTo, connectionId, navigate])

  // What this connection's backend can do: the tabs and actions follow it.
  const capabilities = useCapabilities(connectionId)
  const { can, jobStates } = capabilities
  const known = capabilities.descriptor !== undefined
  const keeps = (state: JobState) => jobStates.some((s) => s.code === state)
  const hasView = (view: QueueTabView) => {
    const need = VIEW_NEEDS[view]
    return need === null || can(need)
  }

  // The Schedulers tab is there whenever the backend lists schedulers (it
  // is not hidden while empty): it is also where a scheduler is created.
  // It carries the count.
  const { data: cronJobs } = useCronJobs({
    connectionId,
    queueId,
    enabled: can("schedules.list"),
  })
  const schedulerCount = cronJobs?.length ?? 0

  // Who consumes this queue. Fetched for the whole page (one
  // cheap `CLIENT LIST`): the tab shows the count and the header warns when
  // there is none. `undefined` while loading / when the provider forbids it.
  const { data: workers } = useWorkers({ connectionId, queueId, enabled: can("workers.list") })
  const workerCount = workers?.supported ? workers.workers.length : undefined

  // Add job. Disabled (with the reason) on a locked connection:
  // better than filling a form that will be refused.
  const [addJobOpen, setAddJobOpen] = useState(false)
  const { data: lockState } = useConnectionLockState(connectionId)
  // The toolbar names the connection the queue belongs to.
  const { data: connections } = useConnections()
  const connection = connections?.find((c) => c.id === connectionId)
  const isLocked = lockState === "locked"

  const defaultState: JobState = countOf(queue?.counts, "failed") > 0 ? "failed" : "waiting"
  // Until the backend's states are known, the URL's state stands.
  const describedState = search.state !== undefined && (!known || keeps(search.state))
  const activeState: JobState = describedState && search.state ? search.state : defaultState
  const activeView: QueueTabView = !known || hasView(search.view) ? search.view : "jobs"
  const scope: JobFilterScope = search.fscope
  const filter = { name: search.fname, error: search.ferror }
  const sort: JobSort | null = search.sort
    ? { field: search.sort, descending: search.dir === "desc" }
    : null
  const { value: rememberedPageSize, set: setPageSize } = useSetting(jobPageSize)
  const pageSize: PageSize = search.size ?? rememberedPageSize
  // Every Jobs-tab navigation keeps the list settings it does not change.
  const jobsSearch = (patch: Partial<QueueSearch>) => jobsSearchOf(search, patch)

  // The Queue and View menus for this queue.
  const copy = useCopyToClipboard()
  useCommandHandler("queue.addJob", () => setAddJobOpen(true), {
    enabled: Boolean(queue) && !isLocked && can("jobActions.add"),
  })
  useCommandHandler("queue.copyKey", () => copy(queueId, "Queue ID"))
  // A job opens beside the list; ⌥⌘I hides or shows it.
  const inspectorHidden = useShellStore((s) => s.inspectorHidden)
  const toggleInspector = useShellStore((s) => s.toggleInspector)
  const openJob = (job: Job) =>
    navigate({
      search: (prev) => ({
        ...prev,
        job: /^\d{1,15}$/.test(job.jobRef.id) ? Number(job.jobRef.id) : job.jobRef.id,
        jobQueue: job.jobRef.queueId === queueId ? undefined : job.jobRef.queueId,
      }),
    })
  const closeJob = () =>
    navigate({ search: (prev) => ({ ...prev, job: undefined, jobQueue: undefined }) })
  useCommandHandler("view.toggleInspector", toggleInspector, {
    enabled: search.job !== undefined,
    label: inspectorHidden ? "Show Inspector" : "Hide Inspector",
  })
  const inspector =
    search.job !== undefined && !inspectorHidden ? (
      <JobInspector
        connectionId={connectionId}
        queueId={search.jobQueue ?? queueId}
        jobId={String(search.job)}
        onClose={closeJob}
      />
    ) : undefined
  useCommandHandler("queue.openInNewTab", () =>
    openRouteOrSay(queueRoute(connectionId, queueId), { tab: true }),
  )
  useCommandHandler(
    "view.state.waiting",
    () => navigate({ search: jobsSearch({ state: "waiting" }) }),
    { enabled: keeps("waiting") },
  )
  useCommandHandler(
    "view.state.active",
    () => navigate({ search: jobsSearch({ state: "active" }) }),
    { enabled: keeps("active") },
  )
  useCommandHandler(
    "view.state.delayed",
    () => navigate({ search: jobsSearch({ state: "delayed" }) }),
    { enabled: keeps("delayed") },
  )
  useCommandHandler(
    "view.state.failed",
    () => navigate({ search: jobsSearch({ state: "failed" }) }),
    { enabled: keeps("failed") },
  )
  // View › Group by Cause (⌥⌘G), on the Failed tab.
  const byCause = activeView === "jobs" && activeState === "failed" && search.group === "cause"
  const setGrouping = (next: "list" | "cause") =>
    navigate({
      search: jobsSearch({ state: "failed", group: next === "cause" ? "cause" : undefined }),
    })
  useCommandHandler("view.groupByCause", () => setGrouping(byCause ? "list" : "cause"), {
    enabled: activeView === "jobs" && activeState === "failed",
    label: byCause ? "Show as List" : "Group by Cause",
  })
  const groupToggle = (
    <Segmented
      label="Group the failed jobs"
      value={byCause ? "cause" : "list"}
      onChange={setGrouping}
      options={[
        { value: "list", label: "List", testId: "failed-group-list" },
        { value: "cause", label: "By cause", testId: "failed-group-cause" },
      ]}
    />
  )
  useCommandHandler(
    "view.state.completed",
    () => navigate({ search: jobsSearch({ state: "completed" }) }),
    { enabled: keeps("completed") },
  )

  return (
    <ViewTabsRoot
      // The job states are tabs of their own (`state:<name>`),
      // beside the queue's other views.
      value={activeView === "jobs" ? `state:${activeState}` : activeView}
      onValueChange={(v) => {
        const value = String(v)
        if (value.startsWith("state:")) {
          navigate({ search: jobsSearch({ state: value.slice(6) }) })
        } else {
          navigate({ search: { view: value as QueueTabView, page: 0 } })
        }
      }}
      className="h-full gap-0"
    >
      <Screen
        toolbar={
          <QueueToolbar
            connectionId={connectionId}
            queueName={queueName}
            queueId={queueId}
            queue={queue}
            connection={connection}
            workerCount={workerCount}
            locked={isLocked}
            onAddJob={() => setAddJobOpen(true)}
            onShowWorkers={() => navigate({ search: { view: "workers", page: 0 } })}
          />
        }
        inspector={inspector}
        banner={
          <>
            {/* The selection bar sits above the views while a selection is active. */}
            <BulkActionBar connectionId={connectionId} queueId={queueId} />
          </>
        }
        tabs={
          <ViewTabs
            items={[
              ...visibleStateTabs(
                jobStates,
                queue?.counts,
                activeView === "jobs" ? activeState : undefined,
              ).map((t) => {
                const count = countOf(queue?.counts, t.code)
                return {
                  value: `state:${t.code}`,
                  label: t.label,
                  count: queue ? count.toLocaleString() : undefined,
                  tone: t.code === "failed" && count > 0 ? ("danger" as const) : undefined,
                  testId: `job-state-tab-${t.code}`,
                }
              }),
              ...[
                {
                  value: "cron" as const,
                  label: "Schedulers",
                  count: schedulerCount > 0 ? schedulerCount : undefined,
                  countTestId: "schedulers-tab-count",
                },
                {
                  value: "workers" as const,
                  label: "Workers",
                  count: workerCount,
                  countTestId: "workers-tab-count",
                },
                { value: "flows" as const, label: "Flows" },
                { value: "overview" as const, label: "Metrics" },
              ].filter((tab) => hasView(tab.value)),
            ]}
          />
        }
      >
        <AddJobDialog
          open={addJobOpen}
          onOpenChange={setAddJobOpen}
          connectionId={connectionId}
          queueId={queueId}
          queueName={queueName}
          queuePaused={queue?.isPaused ?? false}
          onAdded={(added) => {
            // Show it where it landed; the id filter singles it out.
            const state = keeps(added.state) ? added.state : "waiting"
            navigate({ search: { view: "jobs", state, page: 0, fname: added.jobId } })
          }}
        />
        <ViewPanel value="overview" className="min-h-0 flex-1 overflow-auto pr-2">
          <QueueOverview
            connectionId={connectionId}
            queueId={queueId}
            range={search.range}
            onRangeChange={(range) => navigate({ search: (prev) => ({ ...prev, range }) })}
          />
        </ViewPanel>
        {byCause ? (
          <FailureCausesView
            connectionId={connectionId}
            queueId={queueId}
            queueName={queueName}
            toggle={groupToggle}
            onOpenJob={openJob}
          />
        ) : activeView === "jobs" ? (
          <div className="flex min-h-0 flex-1 flex-col gap-3">
            <JobsPanel
              connectionId={connectionId}
              queueId={queueId}
              activeState={activeState}
              page={search.page}
              scope={scope}
              filter={filter}
              sort={sort}
              pageSize={pageSize}
              counts={queue?.counts}
              onFilterChange={(f) =>
                navigate({ search: jobsSearch({ fname: f.name, ferror: f.error }), replace: true })
              }
              onScopeChange={(s) =>
                navigate({ search: jobsSearch({ fscope: s === "cross" ? "cross" : undefined }) })
              }
              onPageChange={(p) =>
                navigate({ search: jobsSearch({ state: search.state ?? defaultState, page: p }) })
              }
              onSortChange={(next) =>
                navigate({
                  search: jobsSearch({
                    sort: next?.field,
                    dir: next ? (next.descending ? "desc" : "asc") : undefined,
                  }),
                })
              }
              onPageSizeChange={(size) => {
                setPageSize(size)
                navigate({ search: jobsSearch({ size }) })
              }}
              onOpenJob={openJob}
              filterLeading={activeState === "failed" ? groupToggle : undefined}
            />
          </div>
        ) : null}
        <ViewPanel value="cron" className="min-h-0 flex-1 overflow-auto">
          <CronJobsView
            connectionId={connectionId}
            queueId={queueId}
            queueName={queueName}
            locked={isLocked}
          />
        </ViewPanel>
        <ViewPanel value="flows" className="min-h-0 flex-1 overflow-auto pr-2">
          <FlowsView connectionId={connectionId} queueId={queueId} />
        </ViewPanel>
        <ViewPanel value="workers" className="min-h-0 flex-1 overflow-auto pr-2">
          <WorkersTable connectionId={connectionId} queueId={queueId} />
        </ViewPanel>
      </Screen>
    </ViewTabsRoot>
  )
}
