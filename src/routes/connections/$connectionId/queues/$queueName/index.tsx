import { useQuery } from "@tanstack/react-query"
import { createFileRoute, Link, useNavigate } from "@tanstack/react-router"
import { PauseIcon, PlayIcon } from "lucide-react"
import { useEffect, useState } from "react"
import { z } from "zod"
import { displayChord } from "@/commands/chords"
import { runCommand, useCommandHandler } from "@/commands/handlers"
import { EnvDot } from "@/components/data/Env"
import { Kbd } from "@/components/data/Kbd"
import { Segmented } from "@/components/data/Segmented"
import { Toolbar } from "@/components/data/Toolbar"
import { ViewPanel, ViewTabs, ViewTabsRoot } from "@/components/data/ViewTabs"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { FlowsView } from "@/features/flows/FlowsView"
import { AddJobDialog } from "@/features/jobs/AddJobDialog"
import { BulkActionBar } from "@/features/jobs/BulkActionBar"
import { FailureCausesView } from "@/features/jobs/FailureCausesView"
import { JobInspector } from "@/features/jobs/JobInspector"
import { type JobFilterScope, JobsPanel } from "@/features/jobs/JobsPanel"
import { CHIP_STATES, type ChipState, visibleStateTabs } from "@/features/jobs/jobStates"
import { useJobPageSize } from "@/features/jobs/useJobPageSize"
import type { PageSize } from "@/features/jobs/useJobs"
import { useConnectionEvents } from "@/features/live/useConnectionEvents"
import { QueueOverview } from "@/features/metrics/QueueOverview"
import { DEFAULT_METRICS_RANGE, METRICS_RANGE_VALUES } from "@/features/metrics/ranges"
import { CronJobsView } from "@/features/queues/CronJobsView"
import { QueueMaintenanceMenu } from "@/features/queues/QueueMaintenanceMenu"
import { useCronJobs } from "@/features/queues/useCronJobs"
import { usePauseQueue, useResumeQueue } from "@/features/queues/useQueueActions"
import { useQueues } from "@/features/queues/useQueues"
import { LockButton } from "@/features/safety/LockButton"
import { useConnectionLockState } from "@/features/safety/useConnectionLockState"
import { RefreshControl } from "@/features/settings/RefreshControl"
import { useWorkers } from "@/features/workers/useWorkers"
import { WorkersTable } from "@/features/workers/WorkersTable"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import { Screen } from "@/layouts/Screen"
import { commands, type Job, type JobSort, type JobState } from "@/lib/tauri/bindings"
import { openRouteOrSay, queueRoute } from "@/lib/windows/openRoute"
import { useJobSelectionStore } from "@/stores/useJobSelectionStore"
import { useSelectionStore } from "@/stores/useSelectionStore"
import { useShellStore } from "@/stores/useShellStore"

/**
 * Drill-into-queue route.
 *
 * Two top-level views via `view`:
 *   - `?view=jobs` — state chips + filter
 *     input + JobList. In queue scope `fname` (job id or
 *     name), `ferror` (failure reason), `sort`/`dir` and `size` are
 *     sent to `list_jobs` and run across the whole state; the scope
 *     toggle (`fscope=cross`) widens to all queues of the connection
 *     via `useCrossQueueJobs`, filtered client-side.
 *   - `?view=cron` — CronJobsView.
 *   - `?view=workers` — WorkersTable. The tab carries the live
 *     worker count, and the header warns when nothing consumes the queue.
 *
 * URL is the source of truth. The Zod schema validates every
 * param; bad values default to safe ones.
 */
const TAB_VIEWS = ["overview", "jobs", "cron", "workers", "flows"] as const
const FILTER_SCOPES = ["queue", "cross"] as const
const SORT_FIELDS = ["name", "queuedAt", "duration"] as const
const SORT_DIRECTIONS = ["asc", "desc"] as const
type TabView = (typeof TAB_VIEWS)[number]
type FilterScope = JobFilterScope

const searchSchema = z.object({
  // A queue opens on its jobs (the state that needs you first).
  view: z.enum(TAB_VIEWS).default("jobs").catch("jobs"),
  state: z
    .enum(CHIP_STATES as readonly [ChipState, ...ChipState[]])
    .optional()
    .catch(undefined),
  page: z.number().int().min(0).default(0).catch(0),
  fname: z.string().default("").catch(""),
  ferror: z.string().default("").catch(""),
  fscope: z.enum(FILTER_SCOPES).default("queue").catch("queue"),
  // server-side sort + page size of the Jobs tab.
  sort: z.enum(SORT_FIELDS).optional().catch(undefined),
  dir: z.enum(SORT_DIRECTIONS).optional().catch(undefined),
  size: z
    .union([z.literal(25), z.literal(50), z.literal(100), z.literal(250)])
    .optional()
    .catch(undefined),
  // Metrics window of the Overview tab.
  range: z.enum(METRICS_RANGE_VALUES).default(DEFAULT_METRICS_RANGE).catch(DEFAULT_METRICS_RANGE),
  // The job open in the inspector (and its queue, in the
  // all-queues scope, when it is not this one).
  // A numeric id travels as a number, so the URL reads `?job=192`, not `?job="192"`.
  job: z.union([z.number(), z.string()]).optional().catch(undefined),
  jobQueue: z.string().optional().catch(undefined),
  // The Failed tab grouped by cause.
  group: z.enum(["cause"]).optional().catch(undefined),
})

export const Route = createFileRoute("/connections/$connectionId/queues/$queueName/")({
  component: QueueDetailComponent,
  validateSearch: searchSchema,
})

function QueueDetailComponent() {
  const { connectionId, queueName } = Route.useParams()
  const search = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  const select = useSelectionStore((s) => s.select)

  useEffect(() => {
    select(connectionId)
    return () => {
      select(null)
    }
  }, [connectionId, select])

  // Clear the job selection whenever the queue or
  // state-filter changes. Selection scope is "this queue + this
  // state-filter" — switching either erases the selection so the
  // BulkActionBar doesn't carry stale entries across navigation.
  const clearSelection = useJobSelectionStore((s) => s.clear)
  // biome-ignore lint/correctness/useExhaustiveDependencies: queueName + search.state are the intentional triggers (the effect fires when either route param changes)
  useEffect(() => {
    clearSelection()
  }, [queueName, search.state, clearSelection])

  // Follow this connection's event streams while it is on screen.
  useConnectionEvents(connectionId)
  const { data: queues } = useQueues(connectionId)
  const queue = queues?.find((q) => q.queueRef.name === queueName)
  const queueRef = queue?.queueRef ?? { prefix: "bull", name: queueName }

  // The Schedulers tab is always there (it is not hidden while
  // empty): it is also where a scheduler is created. It carries the count.
  const { data: cronJobs } = useCronJobs({ connectionId, queueRef })
  const schedulerCount = cronJobs?.length ?? 0

  // Who consumes this queue. Fetched for the whole page (one
  // cheap `CLIENT LIST`): the tab shows the count and the header warns when
  // there is none. `undefined` while loading / when the provider forbids it.
  const { data: workers } = useWorkers({ connectionId, queueRef })
  const workerCount = workers?.supported ? workers.workers.length : undefined

  // Add job. Disabled (with the reason) on a locked connection:
  // better than filling a form that will be refused.
  const [addJobOpen, setAddJobOpen] = useState(false)
  const { data: lockState } = useConnectionLockState(connectionId)
  // The toolbar names the connection the queue belongs to.
  const { data: connections } = useQuery({
    queryKey: ["connections"],
    queryFn: async () => {
      const r = await commands.listConnections()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
  })
  const connection = connections?.find((c) => c.id === connectionId)
  const isLocked = lockState === "locked"

  const defaultState: ChipState =
    queue?.counters && queue.counters.failed > 0 ? "failed" : "waiting"
  const activeState: JobState = search.state ?? defaultState
  const activeView: TabView = search.view
  const scope: FilterScope = search.fscope
  const filter = { name: search.fname, error: search.ferror }
  const sort: JobSort | null = search.sort
    ? { field: search.sort, descending: search.dir === "desc" }
    : null
  const { pageSize: rememberedPageSize, setPageSize } = useJobPageSize()
  const pageSize: PageSize = search.size ?? rememberedPageSize
  // Every Jobs-tab navigation keeps the list settings it does not change.
  const jobsSearch = (patch: Partial<z.infer<typeof searchSchema>>) => ({
    view: "jobs" as const,
    state: search.state,
    page: 0,
    fname: filter.name || undefined,
    ferror: filter.error || undefined,
    fscope: scope === "cross" ? ("cross" as const) : undefined,
    sort: search.sort,
    dir: search.sort ? search.dir : undefined,
    size: search.size,
    group: search.group,
    ...patch,
  })

  // The Queue and View menus for this queue.
  const copy = useCopyToClipboard()
  useCommandHandler("queue.addJob", () => setAddJobOpen(true), {
    enabled: Boolean(queue) && !isLocked,
  })
  useCommandHandler("queue.copyKey", () => copy(`${queueRef.prefix}:${queueName}`, "Queue key"))
  // A job opens beside the list; ⌥⌘I hides or shows it.
  const inspectorHidden = useShellStore((s) => s.inspectorHidden)
  const toggleInspector = useShellStore((s) => s.toggleInspector)
  const openJob = (job: Job) =>
    navigate({
      search: (prev) => ({
        ...prev,
        job: /^\d{1,15}$/.test(job.jobRef.id) ? Number(job.jobRef.id) : job.jobRef.id,
        jobQueue: job.jobRef.queue.name === queueName ? undefined : job.jobRef.queue.name,
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
        queueRef={{ prefix: queueRef.prefix, name: search.jobQueue ?? queueName }}
        jobId={String(search.job)}
        onClose={closeJob}
      />
    ) : undefined
  useCommandHandler("queue.openInNewTab", () =>
    openRouteOrSay(queueRoute(connectionId, queueName), { tab: true }),
  )
  useCommandHandler("view.state.waiting", () =>
    navigate({ search: jobsSearch({ state: "waiting" }) }),
  )
  useCommandHandler("view.state.active", () =>
    navigate({ search: jobsSearch({ state: "active" }) }),
  )
  useCommandHandler("view.state.delayed", () =>
    navigate({ search: jobsSearch({ state: "delayed" }) }),
  )
  useCommandHandler("view.state.failed", () =>
    navigate({ search: jobsSearch({ state: "failed" }) }),
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
  useCommandHandler("view.state.completed", () =>
    navigate({ search: jobsSearch({ state: "completed" }) }),
  )

  return (
    <ViewTabsRoot
      // The job states are tabs of their own (`state:<name>`),
      // beside the queue's other views.
      value={activeView === "jobs" ? `state:${activeState}` : activeView}
      onValueChange={(v) => {
        const value = String(v)
        if (value.startsWith("state:")) {
          navigate({ search: jobsSearch({ state: value.slice(6) as ChipState }) })
        } else {
          navigate({ search: { view: value as TabView, page: 0 } })
        }
      }}
      className="h-full gap-0"
    >
      <Screen
        toolbar={
          <Toolbar
            leading={
              <Link
                to="/connections/$connectionId"
                params={{ connectionId }}
                className="flex items-center gap-1.5 text-sm text-(--text2) hover:text-(--text) hover:underline"
                title="Back to the connection"
              >
                ←{" "}
                {connection ? (
                  <>
                    <EnvDot color={connection.envColor} label={connection.envLabel} />
                    {connection.name}
                  </>
                ) : (
                  "Queues"
                )}
              </Link>
            }
            title={queueName}
            meta={
              <>
                <span className="hidden font-mono text-xs text-(--text2) lg:inline">
                  {queueRef.prefix}:{queueName}
                </span>
                {queue?.isPaused ? (
                  <span className="rounded-tag bg-(--state-delayed)/15 px-1.5 py-px text-xs text-(--state-delayed-text)">
                    paused
                  </span>
                ) : null}
                {workerCount === 0 ? (
                  <button
                    type="button"
                    onClick={() => navigate({ search: { view: "workers", page: 0 } })}
                    title="No BullMQ worker is connected to this queue: waiting jobs will not be processed."
                    data-testid="queue-no-worker"
                    className="rounded-tag bg-(--state-delayed)/15 px-1.5 py-px text-xs text-(--state-delayed-text) hover:bg-(--state-delayed)/25"
                  >
                    no worker
                  </button>
                ) : null}
              </>
            }
            actions={
              <>
                <LockButton connectionId={connectionId} />
                <RefreshControl connectionId={connectionId} />
                {/* Pause / Resume, disabled while the connection is locked.
                    The toolbar is there from the start (it does not wait for
                    the queue list to arrive); Pause waits for the
                    queue's state, the others only need its name. */}
                <QueuePauseButton
                  connectionId={connectionId}
                  queueRef={queueRef}
                  isPaused={queue?.isPaused ?? false}
                  loading={!queue}
                  locked={isLocked}
                />
                <span>
                  <QueueMaintenanceMenu
                    connectionId={connectionId}
                    queueRef={queueRef}
                    locked={isLocked}
                  />
                </span>
                <TooltipProvider>
                  <Tooltip>
                    <TooltipTrigger
                      render={
                        <span className="inline-block">
                          <Button
                            variant="primary"
                            type="button"
                            size="sm"
                            onClick={() => setAddJobOpen(true)}
                            disabled={isLocked}
                            data-testid="queue-add-job"
                          >
                            Add job
                          </Button>
                        </span>
                      }
                    />
                    <TooltipContent>
                      {isLocked
                        ? "This connection is read-only. Unlock it from the toolbar (⇧⌘U) to add jobs."
                        : "Enqueue a job — workers process it like any other"}
                    </TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              </>
            }
          />
        }
        inspector={inspector}
        banner={
          <>
            {/* The selection bar sits above the views while a selection is active. */}
            <BulkActionBar connectionId={connectionId} queueRef={queueRef} />
          </>
        }
        tabs={
          <ViewTabs
            items={[
              ...visibleStateTabs(
                queue?.counters,
                activeView === "jobs" ? activeState : undefined,
              ).map((t) => {
                const count = Number(queue?.counters?.[t.key] ?? 0)
                return {
                  value: `state:${t.key}`,
                  label: t.label,
                  count: queue ? count.toLocaleString() : undefined,
                  tone: t.key === "failed" && count > 0 ? ("danger" as const) : undefined,
                  testId: `job-state-tab-${t.key}`,
                }
              }),
              {
                value: "cron",
                label: "Schedulers",
                count: schedulerCount > 0 ? schedulerCount : undefined,
                countTestId: "schedulers-tab-count",
              },
              {
                value: "workers",
                label: "Workers",
                count: workerCount,
                countTestId: "workers-tab-count",
              },
              { value: "flows", label: "Flows" },
              { value: "overview", label: "Metrics" },
            ]}
          />
        }
      >
        <AddJobDialog
          open={addJobOpen}
          onOpenChange={setAddJobOpen}
          connectionId={connectionId}
          queueRef={queueRef}
          queuePaused={queue?.isPaused ?? false}
          onAdded={(added) => {
            // Show it where it landed; the id filter singles it out.
            const state = (CHIP_STATES as readonly string[]).includes(added.state)
              ? (added.state as ChipState)
              : "waiting"
            navigate({ search: { view: "jobs", state, page: 0, fname: added.jobId } })
          }}
        />
        <ViewPanel value="overview" className="min-h-0 flex-1 overflow-auto pr-2">
          <QueueOverview
            connectionId={connectionId}
            queueRef={queueRef}
            range={search.range}
            onRangeChange={(range) => navigate({ search: (prev) => ({ ...prev, range }) })}
          />
        </ViewPanel>
        {byCause ? (
          <FailureCausesView
            connectionId={connectionId}
            queueRef={queueRef}
            toggle={groupToggle}
            onOpenJob={openJob}
          />
        ) : activeView === "jobs" ? (
          <div className="flex min-h-0 flex-1 flex-col gap-3">
            <JobsPanel
              connectionId={connectionId}
              queueRef={queueRef}
              activeState={activeState}
              page={search.page}
              scope={scope}
              filter={filter}
              sort={sort}
              pageSize={pageSize}
              counters={queue?.counters}
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
          <CronJobsView connectionId={connectionId} queueRef={queueRef} locked={isLocked} />
        </ViewPanel>
        <ViewPanel value="flows" className="min-h-0 flex-1 overflow-auto pr-2">
          <FlowsView connectionId={connectionId} queueRef={queueRef} />
        </ViewPanel>
        <ViewPanel value="workers" className="min-h-0 flex-1 overflow-auto pr-2">
          <WorkersTable connectionId={connectionId} queueRef={queueRef} />
        </ViewPanel>
      </Screen>
    </ViewTabsRoot>
  )
}

/**
 * Pause / Resume toggle for the queue header. Reads
 * `isPaused` from the cached queue counter (polled every
 * 1s) and flips the button label between Pause and Resume. The
 * `cmd-shift-P` shortcut wires the same toggle from the keyboard.
 *
 * The label flips at once (optimistic, see `useQueueActions`)
 * and the button stays busy until the refreshed queue list confirms it;
 * disabled until the queue's state is known.
 */
function QueuePauseButton({
  connectionId,
  queueRef,
  isPaused,
  loading,
  locked,
}: {
  connectionId: string
  queueRef: { prefix: string; name: string }
  isPaused: boolean
  /** The queue's state is not known yet. */
  loading: boolean
  locked: boolean
}) {
  const pauseMut = usePauseQueue()
  const resumeMut = useResumeQueue()
  const busy = pauseMut.isPending || resumeMut.isPending || loading

  // Queue › Pause / Resume (⇧⌘P), a registry command.
  useCommandHandler(
    "queue.pause",
    () => {
      if (isPaused) resumeMut.mutate({ connectionId, queueRef })
      else pauseMut.mutate({ connectionId, queueRef })
    },
    {
      // Locked: the lock says how to unlock — no refused write, no error toast.
      enabled: !busy && !locked,
      label: `${isPaused ? "Resume" : "Pause"} “${queueRef.name}”`,
    },
  )

  return (
    <Button
      type="button"
      size="sm"
      variant="default"
      onClick={() => runCommand("queue.pause")}
      disabled={busy || locked}
      title={locked ? "This connection is read-only. Unlock it from the toolbar (⇧⌘U)." : undefined}
      data-testid="queue-pause-toggle"
      className="gap-1.5"
    >
      {isPaused ? <PlayIcon aria-hidden="true" /> : <PauseIcon aria-hidden="true" />}
      <span>{isPaused ? "Resume" : "Pause"}</span>
      <Kbd className="hidden xl:inline-flex">{displayChord("cmd+shift+p")}</Kbd>
    </Button>
  )
}
