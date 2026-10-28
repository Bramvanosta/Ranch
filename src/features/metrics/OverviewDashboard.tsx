/**
 * Generic dashboard rendering a `QueueMetrics` payload.
 *
 * Shared by the per-queue Overview tab and the connection-level one (same
 * shape, merged server-side). Layout:
 *
 *   Performance                                   [ Last 24 hours ▾ ]
 *   ┌───────────┬───────────┬───────────┬───────────┐
 *   │ Throughput│ Failure   │ Processing│ Queue     │
 *   │           │ rate      │ time      │ delay     │
 *   └───────────┴───────────┴───────────┴───────────┘
 *   ⚠ estimated-counts notice (when native metrics are not recorded)
 *   ┌───────────────────────┬───────────────────────┐
 *   │ Job throughput        │ Processing performance│
 *   └───────────────────────┴───────────────────────┘
 *   ┌───────────────────────┬───────────────────────┐
 *   │ Slowest jobs          │ Most failing job types│
 *   └───────────────────────┴───────────────────────┘
 *
 * Charts use shadcn's <ChartContainer> over Recharts; theme tokens
 * (`--chart-1` … `--chart-5`) come from globals.css.
 */
import { Link } from "@tanstack/react-router"
import { Bar, BarChart, CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts"

import { Panel, PanelBody, PanelHeader, PanelTitle } from "@/components/data/Panel"
import {
  type ChartConfig,
  ChartContainer,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart"
import {
  formatCount,
  formatDurationMs,
  formatPercent,
  formatThroughputPerMinute,
} from "@/domain/format"
import type { MetricsRange, QueueMetrics } from "@/lib/tauri/bindings"
import { MetricCard } from "./MetricCard"
import { formatBucketTick, rangeLabel } from "./ranges"
import { TimeRangeSelect } from "./TimeRangeSelect"

interface OverviewDashboardProps {
  metrics: QueueMetrics
  /** Needed to link slowest jobs / failing types to their job views. */
  connectionId: string
  range: MetricsRange
  onRangeChange: (range: MetricsRange) => void
  /** Optional label rendered above the dashboard (e.g. queue name). */
  scopeLabel?: string
  /** Optional fetching-state badge handler (parent owns the spinner). */
  isFetching?: boolean
}

const chartConfig: ChartConfig = {
  // The job-state colors, not the monochrome
  // `--chart-*` ramp: "failed" was a dark green next to a light green, and
  // the two timing lines were two more greens.
  completed: { label: "Completed", color: "var(--state-completed)" },
  failed: { label: "Failed", color: "var(--state-failed)" },
  processingMs: { label: "Processing time", color: "var(--state-active)" },
  queueDelayMs: { label: "Queue delay", color: "var(--state-delayed)" },
}

/** One row per bucket, oldest first. */
export function buildSeries(metrics: QueueMetrics) {
  return metrics.bucketStartsMs.map((startMs, i) => ({
    startMs,
    label: formatBucketTick(startMs, metrics.range),
    completed: metrics.completedSeries[i] ?? 0,
    failed: metrics.failedSeries[i] ?? 0,
    processingMs: metrics.processingTimeSeriesMs[i] ?? 0,
    queueDelayMs: metrics.queueDelaySeriesMs[i] ?? 0,
  }))
}

function minMaxHint(min: number, max: number, sampled: number): string {
  if (sampled === 0) return "no completed jobs in this range"
  return `min ${formatDurationMs(min)} · max ${formatDurationMs(max)}`
}

export function OverviewDashboard({
  metrics,
  connectionId,
  range,
  onRangeChange,
  scopeLabel,
  isFetching,
}: OverviewDashboardProps) {
  const series = buildSeries(metrics)
  const window = rangeLabel(metrics.range).toLowerCase()
  const failureColor =
    metrics.failureRate >= 0.1
      ? "text-(--state-failed-text)"
      : metrics.failureRate > 0
        ? "text-(--state-delayed-text)"
        : undefined
  const estimatedQueues = metrics.totalQueues - metrics.nativeQueues

  return (
    <section
      className="flex flex-col gap-4"
      aria-label={scopeLabel ? `Overview for ${scopeLabel}` : "Overview"}
      data-testid="overview-dashboard"
    >
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-medium">
          Performance
          {isFetching ? (
            <span className="ml-2 text-xs font-normal text-(--text2)">loading the new range</span>
          ) : null}
        </h3>
        <TimeRangeSelect value={range} onChange={onRangeChange} />
      </div>

      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <MetricCard
          label="Throughput"
          value={formatThroughputPerMinute(metrics.throughputPerMinute)}
          hint={`${formatCount(metrics.completedTotal)} completed · ${window}`}
          sparkline={metrics.completedSeries}
          sparklineColor="var(--state-completed)"
        />
        <MetricCard
          label="Failure rate"
          value={formatPercent(metrics.failureRate)}
          hint={`${formatCount(metrics.failedTotal)} failed · ${window}`}
          valueClassName={failureColor}
          sparkline={metrics.failedSeries}
          sparklineColor="var(--state-failed)"
        />
        <MetricCard
          label="Processing time"
          value={formatDurationMs(metrics.avgProcessingTimeMs)}
          hint={minMaxHint(
            metrics.minProcessingTimeMs,
            metrics.maxProcessingTimeMs,
            metrics.sampledJobs,
          )}
          sparkline={metrics.processingTimeSeriesMs}
          sparklineColor="var(--state-active)"
        />
        <MetricCard
          label="Queue delay"
          value={formatDurationMs(metrics.avgQueueDelayMs)}
          hint={minMaxHint(metrics.minQueueDelayMs, metrics.maxQueueDelayMs, metrics.sampledJobs)}
          sparkline={metrics.queueDelaySeriesMs}
          sparklineColor="var(--state-delayed)"
        />
      </div>

      {/* Say when the counts are an estimate, and why. */}
      {metrics.totalQueues > 0 && estimatedQueues > 0 ? (
        <p
          role="note"
          className="rounded-md border border-(--state-delayed)/30 bg-(--state-delayed)/10 px-3 py-2 text-xs text-(--state-delayed-text)"
          data-testid="metrics-estimate-notice"
        >
          {metrics.totalQueues === 1
            ? "This queue does not record BullMQ metrics"
            : `${estimatedQueues} of ${metrics.totalQueues} queues do not record BullMQ metrics`}
          : throughput and failure rate are estimated from the jobs still in Redis, and undercount
          when finished jobs are removed (<code>removeOnComplete</code>). Create the Worker with the{" "}
          <code>metrics</code> option for exact figures.
        </p>
      ) : null}

      <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        <Panel data-testid="overview-throughput-chart">
          <PanelHeader className="pb-1">
            <PanelTitle className="text-sm font-medium">Job throughput</PanelTitle>
            <p className="text-xs text-(--text2)">Completed vs failed · {window}</p>
          </PanelHeader>
          <PanelBody>
            <ChartContainer config={chartConfig} className="h-48 w-full">
              <BarChart data={series} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
                <CartesianGrid vertical={false} strokeDasharray="3 3" />
                <XAxis
                  dataKey="label"
                  tickLine={false}
                  axisLine={false}
                  tickMargin={6}
                  minTickGap={24}
                />
                <YAxis tickLine={false} axisLine={false} width={32} allowDecimals={false} />
                <ChartTooltip cursor={false} content={<ChartTooltipContent indicator="dot" />} />
                <ChartLegend content={<ChartLegendContent />} />
                <Bar
                  dataKey="completed"
                  stackId="t"
                  fill="var(--color-completed)"
                  radius={[2, 2, 0, 0]}
                />
                <Bar
                  dataKey="failed"
                  stackId="t"
                  fill="var(--color-failed)"
                  radius={[2, 2, 0, 0]}
                />
              </BarChart>
            </ChartContainer>
          </PanelBody>
        </Panel>

        <Panel data-testid="overview-processing-chart">
          <PanelHeader className="pb-1">
            <PanelTitle className="text-sm font-medium">Processing performance</PanelTitle>
            <p className="text-xs text-(--text2)">
              Average processing time and queue delay · {window}
            </p>
          </PanelHeader>
          <PanelBody>
            <ChartContainer config={chartConfig} className="h-48 w-full">
              <LineChart data={series} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
                <CartesianGrid vertical={false} strokeDasharray="3 3" />
                <XAxis
                  dataKey="label"
                  tickLine={false}
                  axisLine={false}
                  tickMargin={6}
                  minTickGap={24}
                />
                <YAxis
                  tickLine={false}
                  axisLine={false}
                  width={48}
                  tickFormatter={(v: number) => formatDurationMs(v)}
                />
                <ChartTooltip
                  cursor={false}
                  content={
                    <ChartTooltipContent
                      indicator="line"
                      formatter={(value) =>
                        typeof value === "number" ? formatDurationMs(value) : String(value)
                      }
                    />
                  }
                />
                <ChartLegend content={<ChartLegendContent />} />
                <Line
                  type="monotone"
                  dataKey="processingMs"
                  stroke="var(--color-processingMs)"
                  strokeWidth={2}
                  dot={false}
                />
                <Line
                  type="monotone"
                  dataKey="queueDelayMs"
                  stroke="var(--color-queueDelayMs)"
                  strokeWidth={2}
                  dot={false}
                />
              </LineChart>
            </ChartContainer>
          </PanelBody>
        </Panel>
      </div>

      {/* Top tables. */}
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        <Panel data-testid="slowest-jobs">
          <PanelHeader className="pb-1">
            <PanelTitle className="text-sm font-medium">Slowest jobs</PanelTitle>
            <p className="text-xs text-(--text2)">
              Top {metrics.slowestJobs.length || 10} by processing time
              {metrics.sampledJobs > 0 ? ` · among the last ${metrics.sampledJobs} completed` : ""}
            </p>
          </PanelHeader>
          <PanelBody>
            {metrics.slowestJobs.length === 0 ? (
              <p className="py-6 text-center text-sm text-(--text2)">
                No completed jobs in this time range
              </p>
            ) : (
              <ul className="flex flex-col divide-y divide-(--line)">
                {metrics.slowestJobs.map((job) => (
                  <li
                    key={`${job.queueId}\u0000${job.jobId}`}
                    className="flex items-center justify-between gap-3 py-1.5 text-sm"
                  >
                    <Link
                      to="/connections/$connectionId/queues/$queueId/$jobId"
                      params={{ connectionId, queueId: job.queueId, jobId: job.jobId }}
                      className="min-w-0 flex-1 break-words hover:underline"
                    >
                      <span className="font-medium">{job.name || "(unnamed)"}</span>
                      <span className="ml-2 font-mono text-xs text-(--text2)">
                        {job.queueName} #{job.jobId}
                      </span>
                    </Link>
                    <span className="shrink-0 font-tabular">
                      {formatDurationMs(job.processingTimeMs)}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </PanelBody>
        </Panel>

        <Panel data-testid="failing-job-types">
          <PanelHeader className="pb-1">
            <PanelTitle className="text-sm font-medium">Most failing job types</PanelTitle>
            <p className="text-xs text-(--text2)">Grouped by job name · {window}</p>
          </PanelHeader>
          <PanelBody>
            {metrics.failingJobTypes.length === 0 ? (
              <p className="py-6 text-center text-sm text-(--text2)">
                No failed jobs in this time range
              </p>
            ) : (
              <ul className="flex flex-col divide-y divide-(--line)">
                {metrics.failingJobTypes.map((type) => (
                  <li key={`${type.queueId}\u0000${type.name}`} className="py-1.5 text-sm">
                    <div className="flex items-center justify-between gap-3">
                      <Link
                        to="/connections/$connectionId/queues/$queueId"
                        params={{ connectionId, queueId: type.queueId }}
                        search={{ view: "jobs", state: "failed", fname: type.name, page: 0 }}
                        className="min-w-0 flex-1 break-words hover:underline"
                      >
                        <span className="font-medium">{type.name || "(unnamed)"}</span>
                        <span className="ml-2 font-mono text-xs text-(--text2)">
                          {type.queueName}
                        </span>
                      </Link>
                      <span className="shrink-0 font-tabular text-(--state-failed-text)">
                        {formatCount(type.count)} failed
                      </span>
                    </div>
                    {type.lastReason ? (
                      <p className="text-xs break-words text-(--text2)">{type.lastReason}</p>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
          </PanelBody>
        </Panel>
      </div>
    </section>
  )
}
