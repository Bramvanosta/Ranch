/** The demo world's shape, and the error a fake command throws. */
import type {
  ActivityEvent,
  AuditLogEntry,
  Connection,
  CronJob,
  Environment,
  Group,
  Job,
  JobState,
  McpInstallerOverview,
  McpSafetySettings,
  PendingApproval,
  QueueId,
  QueueWorker,
} from "@/lib/tauri/bindings"

export type Args = Record<string, unknown>

export const MIN = 60_000
export const HOUR = 60 * MIN
export const DAY = 24 * HOUR

export class PreviewError {
  constructor(
    public kind: string,
    public message: string,
    public docUrl: string | null = null,
  ) {}
}

// ── the world ────────────────────────────────────────────────────────────
export interface QueueWorld {
  /** Made by the demo world, as a backend makes its queues' ids. */
  id: QueueId
  name: string
  paused: boolean
  jobs: Map<JobState, Job[]>
  schedulers: CronJob[]
  workers: QueueWorker[]
  nextId: number
}

export interface World {
  groups: Group[]
  environments: Environment[]
  connections: Connection[]
  unlocked: Set<string>
  queues: Map<string, QueueWorld[]>
  kv: Map<string, string>
  safety: McpSafetySettings
  audit: AuditLogEntry[]
  activity: ActivityEvent[]
  approvals: PendingApproval[]
  installers: McpInstallerOverview
}
