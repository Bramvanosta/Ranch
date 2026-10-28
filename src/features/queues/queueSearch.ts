import { z } from "zod"

import { DEFAULT_METRICS_RANGE, METRICS_RANGE_VALUES } from "@/features/metrics"

/**
 * The queue screen's URL: which view, and for the Jobs view the state,
 * page, filter, scope, sort and page size. The URL is the source of truth;
 * every bad value falls back to a safe one. A view or state the queue's
 * backend does not have falls back on the page, which knows it.
 */
export const QUEUE_TAB_VIEWS = ["overview", "jobs", "cron", "workers", "flows"] as const
export type QueueTabView = (typeof QUEUE_TAB_VIEWS)[number]

const FILTER_SCOPES = ["queue", "cross"] as const
const SORT_FIELDS = ["name", "queuedAt", "duration"] as const
const SORT_DIRECTIONS = ["asc", "desc"] as const

export const queueSearchSchema = z.object({
  // A queue opens on its jobs (the state that needs you first).
  view: z.enum(QUEUE_TAB_VIEWS).default("jobs").catch("jobs"),
  // A state code: which ones the queue has, its backend says (the page
  // opens the default state for a code it does not describe).
  state: z.string().min(1).max(64).optional().catch(undefined),
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

export type QueueSearch = z.infer<typeof queueSearchSchema>

/**
 * A Jobs-view navigation: it keeps the list settings it does not change and
 * goes back to the first page. Defaults stay out of the URL.
 */
export function jobsSearch(search: QueueSearch, patch: Partial<QueueSearch>) {
  return {
    view: "jobs" as const,
    state: search.state,
    page: 0,
    fname: search.fname || undefined,
    ferror: search.ferror || undefined,
    fscope: search.fscope === "cross" ? ("cross" as const) : undefined,
    sort: search.sort,
    dir: search.sort ? search.dir : undefined,
    size: search.size,
    group: search.group,
    ...patch,
  }
}
