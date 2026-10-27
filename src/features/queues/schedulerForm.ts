import type { CronJob, Schedule, SchedulerSpec } from "@/lib/tauri/bindings"

/**
 * The scheduler sheet, as data (pure, unit-tested). The cron
 * pattern and the timezone are validated by the backend (`preview_schedule`,
 * then `upsert_scheduler`) — the same parser that computes the first run;
 * this module covers the rest: ids, names, the interval, the JSON template.
 */
export type ScheduleKind = "pattern" | "every"
export type IntervalUnit = "seconds" | "minutes" | "hours" | "days"

export const INTERVAL_UNITS: ReadonlyArray<{ value: IntervalUnit; label: string; ms: number }> = [
  { value: "seconds", label: "seconds", ms: 1_000 },
  { value: "minutes", label: "minutes", ms: 60_000 },
  { value: "hours", label: "hours", ms: 3_600_000 },
  { value: "days", label: "days", ms: 86_400_000 },
]

export interface SchedulerFormValues {
  id: string
  jobName: string
  kind: ScheduleKind
  pattern: string
  tz: string
  everyValue: string
  everyUnit: IntervalUnit
  /** JSON text. */
  data: string
}

export function emptySchedulerForm(tz: string): SchedulerFormValues {
  return {
    id: "",
    jobName: "",
    kind: "pattern",
    pattern: "0 9 * * *",
    tz,
    everyValue: "5",
    everyUnit: "minutes",
    data: "{}",
  }
}

/** Largest unit that divides the interval: 900000 ms → 15 minutes. */
export function splitInterval(ms: number): { value: string; unit: IntervalUnit } {
  for (const unit of [...INTERVAL_UNITS].reverse()) {
    if (ms >= unit.ms && ms % unit.ms === 0)
      return { value: String(ms / unit.ms), unit: unit.value }
  }
  return { value: String(ms / 1_000), unit: "seconds" }
}

function prettyJson(text: string | null): string {
  if (!text) return "{}"
  try {
    return JSON.stringify(JSON.parse(text), null, 2)
  } catch {
    return text
  }
}

/** The form for editing an existing scheduler. */
export function schedulerFormFrom(cj: CronJob, fallbackTz: string): SchedulerFormValues {
  const base = emptySchedulerForm(fallbackTz)
  if (cj.pattern) {
    return {
      ...base,
      id: cj.key,
      jobName: cj.name,
      kind: "pattern",
      pattern: cj.pattern,
      // No stored tz = "the worker's local time": keep it that way on edit.
      tz: cj.tz ?? "",
      data: prettyJson(cj.template),
    }
  }
  const { value, unit } = splitInterval(cj.every ?? 60_000)
  return {
    ...base,
    id: cj.key,
    jobName: cj.name,
    kind: "every",
    everyValue: value,
    everyUnit: unit,
    data: prettyJson(cj.template),
  }
}

/** `null` while the interval is not a usable number. */
export function intervalMs(values: Pick<SchedulerFormValues, "everyValue" | "everyUnit">) {
  const n = Number(values.everyValue.trim())
  if (values.everyValue.trim() === "" || !Number.isFinite(n) || n <= 0) return null
  const unit = INTERVAL_UNITS.find((u) => u.value === values.everyUnit)
  return Math.round(n * (unit?.ms ?? 1_000))
}

/** The schedule to preview / save — `null` while it cannot be built. */
export function scheduleOf(values: SchedulerFormValues): Schedule | null {
  if (values.kind === "pattern") {
    const pattern = values.pattern.trim()
    if (pattern === "") return null
    return { kind: "pattern", pattern, tz: values.tz.trim() === "" ? null : values.tz.trim() }
  }
  const everyMs = intervalMs(values)
  return everyMs === null ? null : { kind: "every", everyMs }
}

export type SchedulerFormErrors = Partial<
  Record<"id" | "jobName" | "pattern" | "every" | "data", string>
>

export type SchedulerFormResult =
  | { ok: true; scheduler: SchedulerSpec }
  | { ok: false; errors: SchedulerFormErrors }

export function validateSchedulerForm(values: SchedulerFormValues): SchedulerFormResult {
  const errors: SchedulerFormErrors = {}

  const id = values.id.trim()
  if (id === "") errors.id = "Give the scheduler an id — saving the same id again edits it"
  else if (id.includes(":")) errors.id = "An id cannot contain ':'"
  else if (/\s/.test(id)) errors.id = "An id cannot contain spaces"
  else if (id.length > 128) errors.id = "At most 128 characters"

  const jobName = values.jobName.trim()
  if (jobName === "") errors.jobName = "Name the jobs this scheduler produces"
  else if (jobName.includes(":")) errors.jobName = "A job name cannot contain ':'"

  const schedule = scheduleOf(values)
  if (values.kind === "pattern" && schedule === null) errors.pattern = "Enter a cron pattern"
  if (values.kind === "every") {
    const ms = intervalMs(values)
    if (ms === null) errors.every = "Enter a positive number"
    else if (ms < 1_000) errors.every = "At least 1 second"
  }

  const data = values.data.trim() === "" ? "{}" : values.data.trim()
  try {
    JSON.parse(data)
  } catch (e) {
    errors.data = e instanceof Error ? e.message : "Not valid JSON"
  }

  if (Object.keys(errors).length > 0 || schedule === null) return { ok: false, errors }
  return { ok: true, scheduler: { id, jobName, schedule, data } }
}

/** "every 15 minutes" / "0 9 * * * (Europe/Paris)". */
export function describeSchedule(schedule: Schedule): string {
  if (schedule.kind === "pattern") {
    return schedule.tz ? `${schedule.pattern} (${schedule.tz})` : schedule.pattern
  }
  const { value, unit } = splitInterval(schedule.everyMs)
  return `every ${value} ${value === "1" ? unit.replace(/s$/, "") : unit}`
}
