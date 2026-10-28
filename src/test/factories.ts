import { bullmqConnection, redisEndpoint } from "@/domain/bullmqEndpoint"
import type { Connection, CronJob, Job, Queue, QueueId } from "@/lib/tauri/bindings"

/**
 * Test data with every field filled in, so a test only states what it is
 * about: `aJob({ id: "7", state: "failed" })`.
 */

/** A queue's id as the BullMQ backend makes it. Screens never build one: tests do. */
export function aQueueId(name = "emails", prefix = "bull"): QueueId {
  return `${prefix}:${name}`
}

/** A waiting job on `emails`, queued a minute ago. */
export function aJob(overrides: Partial<Job> & { id?: string; queue?: string } = {}): Job {
  const { id = "1", queue = "emails", ...fields } = overrides
  return {
    jobRef: { queueId: aQueueId(queue), queueName: queue, id },
    name: "send-email",
    state: "waiting",
    data: "{}",
    opts: "{}",
    attemptsMade: 0,
    timestamp: Date.now() - 60_000,
    processedOn: null,
    finishedOn: null,
    delay: null,
    failedReason: null,
    stacktrace: [],
    returnValue: null,
    parentKey: null,
    progress: null,
    ...fields,
  }
}

/** An empty, running queue. */
export function aQueue(name = "emails", overrides: Partial<Queue> = {}): Queue {
  return {
    id: aQueueId(name),
    name,
    counts: {
      waiting: 0,
      active: 0,
      completed: 0,
      failed: 0,
      delayed: 0,
      prioritized: 0,
      paused: 0,
    },
    isPaused: false,
    ...overrides,
  }
}

/** A local connection, open to agents by default. */
export function aConnection(overrides: Partial<Connection> = {}): Connection {
  const id = overrides.id ?? "conn-1"
  return {
    id,
    environmentId: "env-1",
    groupId: "g-1",
    name: `Connection ${id}`,
    ...bullmqConnection(redisEndpoint({ host: "127.0.0.1" })),
    envColor: "env-local",
    envName: "Local",
    envLabel: "Local",
    groupName: "Default",
    createdAt: "2026-05-11T12:00:00Z",
    mcpReadonly: null,
    sidebarCollapsed: false,
    ...overrides,
  }
}

/** A running nightly cron scheduler, due in an hour. */
export function aCronJob(overrides: Partial<CronJob> = {}): CronJob {
  return {
    key: "nightly",
    name: "report",
    pattern: "0 3 * * *",
    every: null,
    tz: null,
    offset: null,
    iterationCount: 0,
    nextRun: Date.now() + 3_600_000,
    endDate: null,
    template: null,
    paused: false,
    ...overrides,
  }
}
