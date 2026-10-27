import type { QueueMetrics } from "@/lib/tauri/bindings"

/** Test fixture — an empty 24h `QueueMetrics` (24 hourly buckets), overridable. */
export function emptyMetrics(overrides: Partial<QueueMetrics> = {}): QueueMetrics {
  const zeros = Array(24).fill(0) as number[]
  const now = Date.UTC(2026, 8, 18, 12, 0, 0)
  return {
    range: "24h",
    bucketMs: 3_600_000,
    bucketStartsMs: zeros.map((_, i) => now - (24 - i) * 3_600_000),
    completedTotal: 0,
    failedTotal: 0,
    failureRate: 0,
    throughputPerMinute: 0,
    completedSeries: zeros,
    failedSeries: zeros,
    avgProcessingTimeMs: 0,
    minProcessingTimeMs: 0,
    maxProcessingTimeMs: 0,
    avgQueueDelayMs: 0,
    minQueueDelayMs: 0,
    maxQueueDelayMs: 0,
    processingTimeSeriesMs: zeros,
    queueDelaySeriesMs: zeros,
    sampledJobs: 0,
    nativeQueues: 1,
    totalQueues: 1,
    slowestJobs: [],
    failingJobTypes: [],
    ...overrides,
  }
}
