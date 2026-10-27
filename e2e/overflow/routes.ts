// The screens the overflow check sweeps, on the web preview's demo world
// (`src/dev/webPreview.ts`). A story that adds a screen adds its route here.

export interface SweepRoute {
  name: string
  path: string
}

export const ROUTES: SweepRoute[] = [
  { name: "home", path: "/" },
  { name: "connection-overview", path: "/connections/c-prod?view=overview" },
  // Every queue on one time axis.
  { name: "connection-timeline", path: "/connections/c-prod?view=timeline" },
  // The Map, and 42 queues with "Only queues with activity" off.
  { name: "connection-map", path: "/connections/c-local?view=map" },
  { name: "connection-map-42", path: "/connections/c-staging?view=map&queues=many&all=1" },
  { name: "connection-queues", path: "/connections/c-prod?view=queues" },
  { name: "connection-failed", path: "/connections/c-prod?view=failed" },
  { name: "connection-unreachable", path: "/connections/c-side" },
  // The connection's flows, the blocked one open.
  { name: "connection-flows", path: "/connections/c-local?view=flows" },
  {
    name: "connection-flow-inspector",
    path: "/connections/c-local?view=flows&flow=orders%2Fflow-1",
  },
  // The connection's workers, one open in the inspector.
  { name: "connection-workers", path: "/connections/c-local?view=workers" },
  {
    name: "connection-worker-inspector",
    path: "/connections/c-local?view=workers&worker=emails%2F112",
  },
  // The connection's schedulers, one open in the inspector.
  { name: "connection-schedulers", path: "/connections/c-local?view=schedulers" },
  {
    name: "connection-scheduler-inspector",
    path: "/connections/c-local?view=schedulers&scheduler=emails%2Fdaily-report",
  },
  { name: "queue-overview", path: "/connections/c-prod/queues/emails?view=overview" },
  { name: "queue-jobs-failed", path: "/connections/c-prod/queues/emails?view=jobs&state=failed" },
  {
    name: "queue-jobs-completed",
    path: "/connections/c-prod/queues/emails?view=jobs&state=completed",
  },
  // Running jobs: progress, usual time, worker.
  { name: "queue-jobs-active", path: "/connections/c-prod/queues/emails?view=jobs&state=active" },
  { name: "queue-schedulers", path: "/connections/c-prod/queues/emails?view=cron" },
  { name: "queue-workers", path: "/connections/c-prod/queues/emails?view=workers" },
  // The only flow of the demo world lives on c-local.
  { name: "queue-flows", path: "/connections/c-local/queues/orders?view=flows" },
  // A job open in the inspector, beside the list.
  {
    name: "queue-job-inspector",
    path: "/connections/c-local/queues/emails?view=jobs&state=failed&job=192",
  },
  // The failed jobs grouped by cause.
  {
    name: "queue-failed-by-cause",
    path: "/connections/c-prod/queues/emails?view=jobs&state=failed&group=cause",
  },
  // IDs 1081–1302 of c-prod/emails are failed jobs.
  { name: "job", path: "/connections/c-prod/queues/emails/1232" },
  // Everything that needs the user, across connections.
  { name: "attention", path: "/attention" },
  { name: "audit", path: "/audit" },
  // The Agents page.
  { name: "agents-approvals", path: "/agents?tab=approvals" },
  { name: "agents-activity", path: "/agents?tab=activity" },
  { name: "agents-notes", path: "/agents?tab=notes" },
  { name: "agents-setup", path: "/agents?tab=setup" },
  // Settings and its toolbar tabs (a page in the browser).
  { name: "settings-general", path: "/settings?tab=general" },
  { name: "settings-appearance", path: "/settings?tab=appearance" },
  { name: "settings-refresh", path: "/settings?tab=refresh" },
  { name: "settings-alerts", path: "/settings?tab=alerts" },
  { name: "settings-updates", path: "/settings?tab=updates" },
  { name: "settings-advanced", path: "/settings?tab=advanced" },
  { name: "settings-ai", path: "/settings?tab=ai" },
  { name: "first-run", path: "/?preview=empty" },
]

export const THEMES = ["dark", "light"] as const
export type SweepTheme = (typeof THEMES)[number]

export const SIZES = [
  { width: 1440, height: 900 },
  { width: 1100, height: 700 },
] as const

export const sizeLabel = (size: { width: number; height: number }) => `${size.width}x${size.height}`
