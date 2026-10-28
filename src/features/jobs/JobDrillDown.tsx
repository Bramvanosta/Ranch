import { Link } from "@tanstack/react-router"
import { CopyIcon } from "lucide-react"
import { useEffect, useMemo, useState } from "react"
import { useJobStates } from "@/capabilities"
import { useCommandHandler } from "@/commands/handlers"
import { CodeBlock } from "@/components/data/CodeBlock"
import { JsonViewer } from "@/components/data/JsonViewer"
import { Segmented } from "@/components/data/Segmented"
import { StatePill } from "@/components/data/StatePill"
import { TraceBar } from "@/components/data/TraceBar"
import { ViewPanel, ViewTabs, ViewTabsRoot } from "@/components/data/ViewTabs"
import { Button, buttonVariants } from "@/components/ui/button"
import { ProgressBar } from "@/components/ui/progress-bar"
import { Skeleton } from "@/components/ui/skeleton"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useConnection } from "@/data/connections/queries"
import { useFlow } from "@/data/flows/queries"
import { useJob, useJobLogs } from "@/data/jobs/queries"
import { errorKind } from "@/data/result"
import { formatDurationMs } from "@/domain/format"
import { AskAiMenu, useAskAi, useAskAiDefaultAgent, useCodeFolder } from "@/features/askai"
import { JobFlowTab } from "@/features/flows"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import type { Job, QueueId } from "@/lib/tauri/bindings"
import { toYaml } from "@/lib/toYaml"
import { cn } from "@/lib/utils"
import { jobRoute, openRouteOrSay } from "@/lib/windows/openRoute"
import { useProgressStall, useUsualDurations } from "./ActiveJobCells"
import { buildJobTrace } from "./buildJobTrace"
import { JobActions } from "./JobActions"
import { JobDetails, type JobOptions } from "./JobDetails"
import { failureStatus, JobFailureBanner } from "./JobFailureBanner"
import { jobDurationMs } from "./jobDuration"
import { summarizeError } from "./jobPreview"
import { parseJobProgress } from "./parseJobProgress"
import { groupAttempts, StackTraceViewer } from "./StackTraceViewer"

/**
 * Full single-job inspection view, in the inspector beside
 * the list or on the job's own page. It has
 * the inspector's shape:
 *
 *   - **header**: the name, its state, its id (click = copy), the last
 *     error in mono with where the retries stand, and the actions: Retry,
 *     Remove, Copy JSON;
 *   - **tabs**: Trace (the job's life on one time axis, then its facts;
 *     the default for a failed job), Data (payload and result), Error,
 *     Logs n, Raw (the whole job as JSON or YAML), Flow when it is part of
 *     one.
 *
 * The active tab is reflected in the URL via the route's `?tab=` search
 * param so deep links work. Links from before keep working:
 * `payload` / `result` open Data, `stack` Error, `options` Raw, `stats` /
 * `attempts` Trace.
 *
 * Error states:
 *   - `kind === "jobNotFound"` → 404-style empty state + back link.
 *   - `kind === "schemaNotRecognized"` → verbatim message.
 *   - Other → "Cannot reach Redis" + Retry.
 */
export type DrillDownTab =
  | "trace"
  | "data"
  | "error"
  | "logs"
  | "raw"
  | "flow" // only offered for a job that is part of a flow
  // Old link format: mapped to the tabs above.
  | "payload"
  | "result"
  | "stack"
  | "options"
  | "stats"
  | "attempts"

const LEGACY_TABS: Partial<Record<DrillDownTab, DrillDownTab>> = {
  payload: "data",
  result: "data",
  stack: "error",
  options: "raw",
  stats: "trace",
  attempts: "trace",
}

interface JobDrillDownProps {
  connectionId: string
  queueId: QueueId
  jobId: string
  tab?: DrillDownTab
  onTabChange: (tab: DrillDownTab) => void
  /** The job was removed from this page: go somewhere that still exists. */
  onRemoved?: () => void
}

export function JobDrillDown({
  connectionId,
  queueId,
  jobId,
  tab,
  onTabChange,
  onRemoved,
}: JobDrillDownProps) {
  const {
    data: job,
    isLoading,
    isError,
    error,
    refetch,
  } = useJob({
    connectionId,
    queueId,
    jobId,
  })
  // A child knows its parent (`parentKey`); a parent only shows
  // through its flow, so ask. Cheap (one pipeline) and shared with the tab.
  const flow = useFlow({ connectionId, queueId, jobId })
  // One LRANGE, shared with the Logs tab: the tab says how many lines wait there.
  const logs = useJobLogs({ connectionId, queueId, jobId })
  const opts = useMemo<JobOptions>(
    () => (job ? (safeJsonParse(job.opts, {}) as JobOptions) : {}),
    [job],
  )

  if (isLoading) {
    return <DrillDownSkeleton />
  }

  if (isError) {
    return (
      <DrillDownError
        error={error}
        connectionId={connectionId}
        queueId={queueId}
        onRetry={refetch}
      />
    )
  }

  if (!job) {
    // Defensive: TanStack Query's typing allows `data` to be undefined
    // even after isLoading/isError settle in edge cases.
    return <DrillDownSkeleton />
  }

  const hasStack = groupAttempts(job.stacktrace).length > 0
  const hasFailure = hasStack || Boolean(job.failedReason)
  const hasResult = job.returnValue !== null
  const inFlow = job.parentKey !== null || (flow.data?.nodes.length ?? 0) > 1
  const logCount = logs.data?.total ?? 0

  const requested: DrillDownTab | undefined = tab ? (LEGACY_TABS[tab] ?? tab) : undefined
  // A failed job opens on its trace; a running one on its progress (13.5).
  const defaultTab: DrillDownTab =
    job.state === "failed" || job.state === "active" ? "trace" : "data"
  const activeTab: DrillDownTab = requested ?? defaultTab
  const maxAttempts = typeof opts.attempts === "number" ? opts.attempts : null

  return (
    <TooltipProvider>
      <div className="flex min-w-0 flex-col gap-4" data-testid="job-drill-down">
        <JobDrillDownHeader
          job={job}
          connectionId={connectionId}
          maxAttempts={maxAttempts}
          onRemoved={onRemoved}
        />

        <ViewTabsRoot
          value={activeTab}
          onValueChange={(v) => onTabChange(v as DrillDownTab)}
          className="flex flex-col gap-3"
        >
          <ViewTabs
            className="px-0"
            items={[
              { value: "trace", label: job.state === "active" ? "Progress" : "Trace" },
              { value: "data", label: "Data" },
              ...(hasFailure || activeTab === "error" ? [{ value: "error", label: "Error" }] : []),
              {
                value: "logs",
                label: "Logs",
                count: logCount > 0 ? logCount : undefined,
                countTestId: "job-drill-down-log-count",
              },
              { value: "raw", label: "Raw" },
              // Only for a job that is part of a flow.
              ...(inFlow || activeTab === "flow" ? [{ value: "flow", label: "Flow" }] : []),
            ]}
          />
          <ViewPanel value="trace" className="flex flex-col gap-4">
            {job.state === "active" ? (
              <ActiveJobPanel job={job} connectionId={connectionId} />
            ) : (
              <TraceTab job={job} opts={opts} connectionId={connectionId} />
            )}
            <JobDetails job={job} connectionId={connectionId} opts={opts} />
          </ViewPanel>
          <ViewPanel value="data" className="flex flex-col gap-4">
            <PayloadTab job={job} />
            {hasResult || job.state === "completed" ? (
              <section aria-label="Result" className="flex flex-col gap-2">
                <h3 className="text-xs font-semibold text-(--text2)">Result</h3>
                <ResultTab job={job} />
              </section>
            ) : null}
          </ViewPanel>
          <ViewPanel value="error" className="flex flex-col gap-3">
            <JobFailureBanner job={job} maxAttempts={maxAttempts} />
            <StackTraceViewer
              stacktrace={job.stacktrace}
              failedReason={job.failedReason}
              showReason={false}
            />
          </ViewPanel>
          <ViewPanel value="logs">
            <LogsTab
              connectionId={connectionId}
              queueId={queueId}
              jobId={jobId}
              live={job.state === "active"}
            />
          </ViewPanel>
          <ViewPanel value="raw">
            <RawTab job={job} />
          </ViewPanel>
          <ViewPanel value="flow" className="flex min-h-80 flex-col">
            <JobFlowTab connectionId={connectionId} queueId={queueId} jobId={jobId} />
          </ViewPanel>
        </ViewTabsRoot>
      </div>
    </TooltipProvider>
  )
}

function JobDrillDownHeader({
  job,
  connectionId,
  maxAttempts,
  onRemoved,
}: {
  job: Job
  connectionId: string
  maxAttempts: number | null
  onRemoved?: () => void
}) {
  const copy = useCopyToClipboard()
  // Edit › Copy Job ID (⇧⌘C) and Job › Copy Payload (⌥⌘C).
  useCommandHandler("job.copyId", () => copy(job.jobRef.id, "job ID"), { priority: 1 })
  useCommandHandler("job.copyPayload", () => copy(prettyJson(job.data), "payload"))
  useCommandHandler("job.openInNewWindow", () =>
    openRouteOrSay(jobRoute(connectionId, job.jobRef.queueId, job.jobRef.id), { tab: false }),
  )
  const final = job.state === "failed"
  // Ask AI about this job (⇧⌥⌘A runs the default agent).
  const connection = useConnection(connectionId)
  const states = useJobStates(connectionId)
  const askAi = useAskAi()
  const defaultAgent = useAskAiDefaultAgent()
  const folder = useCodeFolder(connection?.groupId)
  const aiOn = useAiEnabled()
  useCommandHandler(
    "agents.askAi",
    () => connection && void askAi({ kind: "job", connection, job }, defaultAgent, folder),
    { enabled: aiOn && Boolean(connection), priority: 2, label: "Ask AI About This Job" },
  )

  return (
    <header className="flex flex-col gap-2" data-testid="job-drill-down-header">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <h1
          className="min-w-0 text-xl font-semibold tracking-tight wrap-anywhere"
          data-testid="job-drill-down-name"
        >
          {job.name || "(unnamed job)"}
        </h1>
        <StatePill state={job.state} states={states} data-testid="job-drill-down-state" />
        {/* The id is its own copy button. */}
        <button
          type="button"
          onClick={() => copy(job.jobRef.id, "job ID")}
          title="Copy the job ID"
          data-testid="job-drill-down-copy-id"
          className="group inline-flex min-w-0 items-center gap-1.5 rounded-tag border border-(--line) bg-(--surf) px-1.5 py-0.5 font-tabular text-xs text-(--text2) hover:text-(--text) focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)/60"
        >
          <span className="wrap-anywhere" data-testid="job-drill-down-id">
            #{job.jobRef.id}
          </span>
          <CopyIcon
            className="size-3 shrink-0 opacity-50 group-hover:opacity-100"
            aria-hidden="true"
          />
        </button>
        {/* One row of buttons that wraps as a whole, in the page as in the
            narrow inspector. */}
        <div className="ml-auto flex flex-wrap items-center gap-1.5">
          {/* ⇧⌘R / ⌘⌫ / ⌥⌘P act here. */}
          <JobActions
            connectionId={connectionId}
            queueId={job.jobRef.queueId}
            job={job}
            onRemoved={onRemoved}
          />
          {connection ? <AskAiMenu target={{ kind: "job", connection, job }} /> : null}
          <Button
            type="button"
            size="sm"
            variant="default"
            onClick={() => copy(JSON.stringify(rawJob(job), null, 2), "job as JSON")}
            data-testid="job-drill-down-copy-json"
          >
            Copy JSON
          </Button>
        </div>
      </div>
      {job.failedReason ? (
        <div className="flex flex-col gap-0.5" data-testid="job-last-error">
          <p
            className={cn(
              "font-tabular text-sm break-words",
              final ? "text-(--state-failed-text)" : "text-(--state-delayed-text)",
            )}
            data-testid="job-last-error-reason"
          >
            {summarizeError(job.failedReason)}
          </p>
          <p className="text-xs text-(--text2)" data-testid="job-last-error-status">
            {failureStatus(job, maxAttempts)}
          </p>
        </div>
      ) : null}
    </header>
  )
}

/** The job's life on one axis, then the facts below it. */
function TraceTab({
  job,
  opts,
  connectionId,
}: {
  job: Job
  opts: JobOptions
  connectionId: string
}) {
  const live = job.state === "active" || job.processedOn === null
  const now = useNow(live)
  const states = useJobStates(connectionId)
  const trace = useMemo(() => buildJobTrace(job, opts, now, states), [job, opts, now, states])
  return (
    <section aria-label="Trace" className="flex flex-col gap-2" data-testid="job-trace">
      <p className="text-sm" data-testid="job-trace-summary">
        {trace.summary}
      </p>
      <TraceBar rows={trace.rows} start={trace.start} end={trace.end} />
      {trace.estimated ? (
        <p className="text-xs text-(--text2)">
          BullMQ keeps the times of the last attempt only: the earlier ones (≈) are placed from the
          job's backoff setting.
        </p>
      ) : null}
    </section>
  )
}

/** BullMQ's default `lockDuration`: what a lock's renewal age is read against. */
const DEFAULT_LOCK_MS = 30_000

/**
 * A running job instead of its trace: its progress and note,
 * how long it runs against the usual time of its type, its worker, its lock
 * and its logs as they come.
 */
function ActiveJobPanel({ job, connectionId }: { job: Job; connectionId: string }) {
  const now = useNow(true)
  const parsed = parseJobProgress(job.progress)
  const { quietMs, stalled } = useProgressStall(connectionId, job, now)
  const usual = useUsualDurations(connectionId, job.jobRef.queueId).get(job.name)
  const running = jobDurationMs(job, now)
  const lock = job.lockTtlMs ?? null
  return (
    <section aria-label="Progress" className="flex flex-col gap-3" data-testid="job-active">
      {parsed?.percent != null ? (
        <ProgressBar
          value={parsed.percent}
          label={parsed.label}
          ariaLabel="Job progress"
          tone={stalled ? "warning" : "active"}
        />
      ) : (
        <p className="text-sm text-(--text2)">
          {parsed ? parsed.label : "This job has not reported any progress."}
        </p>
      )}
      {parsed?.note ? <p className="text-sm">{parsed.note}</p> : null}
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
        <dt className="text-(--text2)">Running</dt>
        <dd className="font-tabular" data-testid="job-active-running">
          {running === null ? "-" : formatDurationMs(running)}
          {usual !== undefined ? (
            <span
              className={cn(
                "ml-2 text-xs",
                running !== null && running > usual
                  ? "text-(--state-delayed-text)"
                  : "text-(--text2)",
              )}
            >
              usual {formatDurationMs(usual)}
            </span>
          ) : null}
        </dd>
        {/* BullMQ keeps no time for a progress update: this is what Ranch saw. */}
        <dt className="text-(--text2)">Progress unchanged</dt>
        <dd className="font-tabular" data-testid="job-active-quiet">
          {quietMs === null ? "-" : `for ${formatDurationMs(quietMs)}`}
          {stalled ? (
            <span className="ml-2 text-xs text-(--state-delayed-text)">no progress for 3 min</span>
          ) : null}
        </dd>
        <dt className="text-(--text2)">Worker</dt>
        <dd className="font-tabular wrap-anywhere">{job.processedBy ?? "-"}</dd>
        <dt className="text-(--text2)">Lock</dt>
        <dd className="font-tabular" data-testid="job-active-lock">
          {lock === null
            ? "not held"
            : lock <= DEFAULT_LOCK_MS
              ? `renewed ≈ ${formatDurationMs(DEFAULT_LOCK_MS - lock)} ago · expires in ${formatDurationMs(lock)}`
              : `expires in ${formatDurationMs(lock)}`}
        </dd>
      </dl>
      {job.progress !== null ? (
        <CodeBlock label="Progress data">
          <code>{prettyJson(job.progress)}</code>
        </CodeBlock>
      ) : null}
      <section aria-label="Logs" className="flex flex-col gap-2">
        <h3 className="text-xs font-semibold text-(--text2)">Logs</h3>
        <LogsTab
          connectionId={connectionId}
          queueId={job.jobRef.queueId}
          jobId={job.jobRef.id}
          live
        />
      </section>
    </section>
  )
}

/** Re-render every second while something on screen depends on the clock. */
function useNow(ticking: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!ticking) return
    const id = setInterval(() => setNow(Date.now()), 1_000)
    return () => clearInterval(id)
  }, [ticking])
  return now
}

/** The whole job, as JSON or YAML, copied as shown. */
function RawTab({ job }: { job: Job }) {
  const [format, setFormat] = useState<"json" | "yaml">("json")
  const copy = useCopyToClipboard()
  const value = useMemo(() => rawJob(job), [job])
  const text = useMemo(
    () => (format === "json" ? JSON.stringify(value, null, 2) : toYaml(value)),
    [format, value],
  )
  return (
    <div className="flex flex-col gap-2" data-testid="job-raw">
      <Segmented
        label="Format"
        value={format}
        onChange={setFormat}
        options={[
          { value: "json", label: "JSON", testId: "job-raw-json" },
          { value: "yaml", label: "YAML", testId: "job-raw-yaml" },
        ]}
        className="self-start"
      />
      <CodeBlock
        label={`The job as ${format.toUpperCase()}`}
        actions={
          <Button
            variant="default"
            size="sm"
            onClick={() => copy(text, `job as ${format.toUpperCase()}`)}
            data-testid="job-raw-copy"
          >
            Copy
          </Button>
        }
      >
        <code data-testid="job-raw-text">{text}</code>
      </CodeBlock>
    </div>
  )
}

/** The job with its JSON fields parsed: what Raw shows and Copy JSON copies. */
function rawJob(job: Job): Record<string, unknown> {
  const parse = (raw: string | null) => (raw === null ? null : safeJsonParse(raw, raw))
  return {
    id: job.jobRef.id,
    queue: job.jobRef.queueName,
    queueId: job.jobRef.queueId,
    name: job.name,
    state: job.state,
    data: parse(job.data),
    opts: parse(job.opts),
    progress: parse(job.progress),
    returnValue: parse(job.returnValue),
    attemptsMade: job.attemptsMade,
    timestamp: job.timestamp,
    processedOn: job.processedOn,
    finishedOn: job.finishedOn,
    delay: job.delay,
    failedReason: job.failedReason,
    stacktrace: job.stacktrace,
    parentKey: job.parentKey,
  }
}

function PayloadTab({ job }: { job: Job }) {
  const parsed = useMemo(() => safeJsonParse(job.data, null), [job.data])
  if (
    job.data === "{}" ||
    parsed === null ||
    (typeof parsed === "object" && Object.keys(parsed as object).length === 0)
  ) {
    return <EmptyTab>This job was added without a payload.</EmptyTab>
  }
  return <JsonViewer value={parsed} copyLabel="payload" />
}

function ResultTab({ job }: { job: Job }) {
  const parsed = useMemo(
    () => (job.returnValue === null ? null : safeJsonParse(job.returnValue, null)),
    [job.returnValue],
  )
  if (job.returnValue === null) {
    const reason =
      job.state === "completed"
        ? "Job completed without emitting a return value."
        : job.state === "failed"
          ? "Job failed, so there is no return value. See the stack trace for the error."
          : "Job has not completed yet: no return value to show."
    return <EmptyTab testId="result-tab-empty">{reason}</EmptyTab>
  }
  return (
    <div data-testid="result-tab">
      <JsonViewer value={parsed} copyLabel="return value" />
    </div>
  )
}

function EmptyTab({ children, testId }: { children: React.ReactNode; testId?: string }) {
  return (
    <p
      className="rounded-md border border-dashed border-(--line) bg-(--surf)/20 p-6 text-center text-sm text-(--text2)"
      data-testid={testId}
    >
      {children}
    </p>
  )
}

function LogsTab({
  connectionId,
  queueId,
  jobId,
  live = false,
}: {
  connectionId: string
  queueId: QueueId
  jobId: string
  live?: boolean
}) {
  const { data, isLoading, isError, error, refetch } = useJobLogs({
    connectionId,
    queueId,
    jobId,
    live,
  })

  if (isLoading) {
    return (
      <div className="flex flex-col gap-1 rounded-md border border-(--line)/40 p-3">
        {[0, 1, 2, 3].map((i) => (
          <Skeleton key={i} className="h-4 w-full" />
        ))}
      </div>
    )
  }

  if (isError) {
    return (
      <div
        role="alert"
        className="flex flex-col gap-2 rounded-md border border-(--danger)/40 bg-(--danger)/10 p-3 text-sm text-(--state-failed-text)"
      >
        <p className="font-medium">Could not load logs</p>
        <p className="text-xs">{error instanceof Error ? error.message : "Unknown error"}</p>
        <Button variant="default" size="sm" onClick={() => refetch()}>
          Retry
        </Button>
      </div>
    )
  }

  if (!data || data.lines.length === 0) {
    return (
      <EmptyTab>
        No logs. A worker writes them with <code className="font-mono">job.log()</code>.
      </EmptyTab>
    )
  }

  return (
    <div
      className="rounded-md border border-(--line)/40 bg-(--surf) py-2 font-mono text-xs leading-relaxed"
      data-testid="logs-tab"
    >
      {data.lines.map((line, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: log lines are stable in this view
        <div key={i} className="flex items-baseline gap-3 px-3 py-0.5 hover:bg-(--surf)/40">
          <span className="w-8 shrink-0 text-right tabular-nums text-(--text2) select-none">
            {i + 1}
          </span>
          {/* Wrapped, not scrolled sideways: a log line is read to its end. */}
          <span className="min-w-0 break-words whitespace-pre-wrap">{line}</span>
        </div>
      ))}
      {data.hasMore ? (
        <p className="px-3 pt-2 text-2xs text-(--text2) italic">
          Showing the first {data.lines.length} of {data.total} lines.
        </p>
      ) : null}
    </div>
  )
}

function DrillDownSkeleton() {
  return (
    <div className="flex flex-col gap-4" data-testid="job-drill-down-loading">
      <div className="flex flex-wrap items-center gap-3">
        <Skeleton className="h-7 w-48" />
        <Skeleton className="h-5 w-16" />
        <Skeleton className="h-5 w-14" />
        <Skeleton className="ml-auto h-8 w-40" />
      </div>
      <Skeleton className="h-28 w-full rounded-xl" />
      <Skeleton className="h-9 w-80" />
      <Skeleton className="h-48 w-full" />
    </div>
  )
}

function DrillDownError({
  error,
  connectionId,
  queueId,
  onRetry,
}: {
  error: unknown
  connectionId: string
  queueId: QueueId
  onRetry: () => void
}) {
  const kind = errorKind(error)
  const message = error instanceof Error ? error.message : "Unknown error"

  if (kind === "jobNotFound") {
    return (
      <div
        className="flex h-full flex-col items-center justify-center gap-3 rounded-md border border-dashed border-(--line) bg-(--surf)/20 p-8 text-center"
        data-testid="job-drill-down-not-found"
      >
        <p className="text-sm font-medium">Job not found.</p>
        <p className="text-xs text-(--text2)">
          It may have been removed since you last viewed the queue.
        </p>
        <Link
          to="/connections/$connectionId/queues/$queueId"
          params={{ connectionId, queueId }}
          className={buttonVariants({ variant: "default", size: "sm" })}
        >
          ← Back to queue
        </Link>
      </div>
    )
  }

  return (
    <div
      role="alert"
      className="flex flex-col gap-2 rounded-md border border-(--danger)/40 bg-(--danger)/10 p-4 text-sm text-(--state-failed-text)"
      data-testid="job-drill-down-error"
    >
      <p className="font-medium">Could not load job</p>
      <p className="text-xs">{message}</p>
      {kind === "schemaNotRecognized" ? null : (
        <Button variant="default" size="sm" onClick={onRetry}>
          Retry
        </Button>
      )}
    </div>
  )
}

function safeJsonParse(s: string, fallback: unknown): unknown {
  try {
    return JSON.parse(s)
  } catch {
    return fallback
  }
}

/** The payload as the user reads it: indented when it is JSON (it always is, `Job.data`). */
function prettyJson(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2)
  } catch {
    return raw
  }
}
