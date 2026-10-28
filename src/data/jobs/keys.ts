import type { JobState, QueueId } from "@/lib/tauri/bindings"

/**
 * Query keys of jobs. Keys nest from the widest to the narrowest, so
 * invalidating `lists(connection, queue)` reaches every page, state and
 * filter of that queue's list: `["jobs", connection, queueId, state, …]`.
 */
export const jobKeys = {
  /** Every job list of a connection. */
  all: (connectionId: string) => ["jobs", connectionId] as const,
  /** Every job list of one queue, whatever its state, page or filter. */
  lists: (connectionId: string, queueId: QueueId) => ["jobs", connectionId, queueId] as const,
  /** One state's lists of a queue (all pages and filters). */
  listsInState: (connectionId: string, queueId: QueueId, state: JobState) =>
    [...jobKeys.lists(connectionId, queueId), state] as const,
  /** Every open job of one queue. */
  details: (connectionId: string, queueId: QueueId) => ["job", connectionId, queueId] as const,
  detail: (connectionId: string, queueId: QueueId, jobId: string) =>
    [...jobKeys.details(connectionId, queueId), jobId] as const,
  logs: (connectionId: string, queueId: QueueId, jobId: string, offset: number, limit: number) =>
    ["jobLogs", connectionId, queueId, jobId, offset, limit] as const,
  /** One state across several queues of a connection. */
  crossQueue: (connectionId: string, state: JobState, queueIds: readonly QueueId[]) =>
    ["crossQueueJobs", connectionId, state, queueIds.join(",")] as const,
  /** Every page of a connection's failed-jobs list. */
  failed: (connectionId: string) => ["failed-jobs", connectionId] as const,
  /** One page of the failed-jobs list, filtered by queue, name and reason. */
  failedPage: (
    connectionId: string,
    filter: { queue: string; name: string; reason: string },
    page: number,
  ) => [...jobKeys.failed(connectionId), filter.queue, filter.name, filter.reason, page] as const,
  /** Every recent-jobs list of a connection. */
  recent: (connectionId: string) => ["recent-jobs", connectionId] as const,
  /** A connection's recent jobs, or one queue's. */
  recentList: (connectionId: string, queueId?: QueueId | null) =>
    [...jobKeys.recent(connectionId), queueId ?? null] as const,
  /** The latest jobs of one queue, to tell what a scheduler or a worker ran. */
  latest: (connectionId: string, queueId: QueueId) =>
    ["recentJobs", connectionId, queueId, 50] as const,
  /** Every failure-cause list of a connection. */
  failureCauses: (connectionId: string) => ["failureCauses", connectionId] as const,
  /** The causes of one queue (by name), or of the whole connection (`""`). */
  failureCauseList: (connectionId: string, queueName: string) =>
    [...jobKeys.failureCauses(connectionId), queueName] as const,
  /** Every job list of one queue's causes. */
  causeJobsOf: (connectionId: string, queueName: string) =>
    ["failureCauseJobs", connectionId, queueName] as const,
  causeJobs: (connectionId: string, queueName: string, signature: string) =>
    [...jobKeys.causeJobsOf(connectionId, queueName), signature] as const,
  /** A job id looked up across the queues listed (the palette's `#1211`). */
  search: (connectionId: string | null, jobId: string | null) =>
    ["palette.job", connectionId, jobId] as const,
}
