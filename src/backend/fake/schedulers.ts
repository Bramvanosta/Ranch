import type { CronJob } from "@/lib/tauri/bindings"
import type { FakeHandlers } from "./handlers"
import { type Args, DAY, PreviewError } from "./types"
import { a, audit, guardWrite, nextRuns, queueOf } from "./world"

/** Job schedulers: list, preview, runs in a window, upsert, pause, remove. */
export const schedulersHandlers = {
  // schedulers
  list_cron_jobs: (args) => queueOf(a(args).connectionId, a(args).queueId).schedulers,
  preview_schedule: ({ schedule }) => ({ nextRunsMs: nextRuns(schedule as Args) }),
  // Runs in a window. Minute by minute against the pattern's
  // fields (*, */n, n, a-b, lists): a preview, not a cron engine.
  schedule_runs_within: (args) => {
    const input = a(args)
    const schedule = input.schedule as Args
    const end = Date.now() + Number(input.horizonMs ?? DAY)
    const max = Math.min(Number(input.max ?? 500), 500)
    const runs: number[] = []
    if (schedule.kind === "every") {
      const every = Number(schedule.everyMs)
      let t = Number(input.firstMs ?? Date.now() + every)
      while (t <= Date.now()) t += every
      while (t <= end && runs.length < max) {
        runs.push(t)
        t += every
      }
      return { nextRunsMs: runs }
    }
    const fields = String(schedule.pattern ?? "")
      .trim()
      .split(/\s+/)
    const [m, h, dom, mon, dow] = fields.length === 6 ? fields.slice(1) : fields
    const match = (field: string | undefined, v: number) =>
      (field ?? "*").split(",").some((part) => {
        const [range, step] = part.split("/")
        const n = Number(step ?? 1)
        if (range === "*") return v % n === 0
        const [lo, hi] = (range ?? "").split("-").map(Number)
        return v >= (lo ?? 0) && v <= (hi ?? lo ?? 0) && (v - (lo ?? 0)) % n === 0
      })
    const t = new Date(Date.now())
    t.setSeconds(0, 0)
    for (
      t.setMinutes(t.getMinutes() + 1);
      t.getTime() <= end && runs.length < max;
      t.setMinutes(t.getMinutes() + 1)
    ) {
      if (
        match(m, t.getMinutes()) &&
        match(h, t.getHours()) &&
        match(dom, t.getDate()) &&
        match(mon, t.getMonth() + 1) &&
        match(dow, t.getDay())
      ) {
        runs.push(t.getTime())
      }
    }
    return { nextRunsMs: runs }
  },
  upsert_scheduler: (args) => {
    const input = a(args)
    guardWrite(input.connectionId)
    const q = queueOf(input.connectionId, input.queueId)
    const spec = input.scheduler as { id: string; jobName: string; schedule: Args; data: string }
    const runs = nextRuns(spec.schedule)
    const existing = q.schedulers.find((s) => s.key === spec.id)
    const next: CronJob = {
      key: spec.id,
      name: spec.jobName,
      pattern: spec.schedule.kind === "pattern" ? String(spec.schedule.pattern) : null,
      every: spec.schedule.kind === "every" ? Number(spec.schedule.everyMs) : null,
      tz: spec.schedule.kind === "pattern" ? ((spec.schedule.tz as string | null) ?? null) : null,
      offset: 0,
      iterationCount: existing?.iterationCount ?? 1,
      nextRun: runs[0] as number,
      endDate: null,
      template: spec.data === "{}" ? null : spec.data,
      paused: false,
    }
    q.schedulers = existing
      ? q.schedulers.map((s) => (s.key === spec.id ? next : s))
      : [...q.schedulers, next]
    audit(String(input.connectionId), "upsertScheduler", {
      kind: "scheduler",
      data: { queueId: q.id, queueName: q.name, schedulerKey: spec.id },
    })
    return {
      schedulerId: spec.id,
      nextRunMs: next.nextRun,
      jobId: `repeat:${spec.id}:${next.nextRun}`,
    }
  },
  remove_scheduler: (args) => {
    const input = a(args)
    guardWrite(input.connectionId)
    const q = queueOf(input.connectionId, input.queueId)
    q.schedulers = q.schedulers.filter((s) => s.key !== input.schedulerKey)
    audit(String(input.connectionId), "removeScheduler", {
      kind: "scheduler",
      data: { queueId: q.id, queueName: q.name, schedulerKey: String(input.schedulerKey) },
    })
    return { paused: false, alreadyInState: false }
  },
  pause_scheduler: (args) => toggleScheduler(a(args), true),
  resume_scheduler: (args) => toggleScheduler(a(args), false),
} satisfies FakeHandlers

function toggleScheduler(input: Args, paused: boolean) {
  guardWrite(input.connectionId)
  const q = queueOf(input.connectionId, input.queueId)
  const s = q.schedulers.find((x) => x.key === input.schedulerKey)
  if (!s)
    throw new PreviewError(
      "schedulerNotFound",
      `Scheduler not found: ${String(input.schedulerKey)}`,
    )
  const already = s.paused === paused
  s.paused = paused
  return { paused, alreadyInState: already }
}
