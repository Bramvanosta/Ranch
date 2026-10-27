import { describe, expect, it } from "vitest"

import type { CronJob } from "@/lib/tauri/bindings"

import {
  describeSchedule,
  emptySchedulerForm,
  intervalMs,
  scheduleOf,
  schedulerFormFrom,
  splitInterval,
  validateSchedulerForm,
} from "./schedulerForm"

const cron = (over: Partial<CronJob>): CronJob => ({
  key: "nightly",
  name: "build-report",
  pattern: null,
  every: null,
  tz: null,
  offset: null,
  iterationCount: 3,
  nextRun: 0,
  endDate: null,
  template: null,
  paused: false,
  ...over,
})

describe("intervals", () => {
  it("splits into the largest unit that divides it", () => {
    expect(splitInterval(900_000)).toEqual({ value: "15", unit: "minutes" })
    expect(splitInterval(86_400_000)).toEqual({ value: "1", unit: "days" })
    expect(splitInterval(90_000)).toEqual({ value: "90", unit: "seconds" })
    expect(splitInterval(1_500)).toEqual({ value: "1.5", unit: "seconds" })
  })

  it("reads value × unit, and nothing else", () => {
    expect(intervalMs({ everyValue: "15", everyUnit: "minutes" })).toBe(900_000)
    expect(intervalMs({ everyValue: "0.5", everyUnit: "hours" })).toBe(1_800_000)
    expect(intervalMs({ everyValue: "", everyUnit: "minutes" })).toBeNull()
    expect(intervalMs({ everyValue: "-3", everyUnit: "minutes" })).toBeNull()
    expect(intervalMs({ everyValue: "soon", everyUnit: "minutes" })).toBeNull()
  })
})

describe("schedulerFormFrom", () => {
  it("loads a cron scheduler, keeping 'no timezone' as it is", () => {
    const form = schedulerFormFrom(
      cron({ pattern: "0 9 * * *", template: '{"kind":"summary"}' }),
      "Europe/Paris",
    )
    expect(form).toMatchObject({
      id: "nightly",
      jobName: "build-report",
      kind: "pattern",
      pattern: "0 9 * * *",
      tz: "",
      data: '{\n  "kind": "summary"\n}',
    })
    expect(schedulerFormFrom(cron({ pattern: "0 9 * * *", tz: "UTC" }), "Europe/Paris").tz).toBe(
      "UTC",
    )
  })

  it("loads an interval scheduler", () => {
    expect(schedulerFormFrom(cron({ every: 900_000 }), "UTC")).toMatchObject({
      kind: "every",
      everyValue: "15",
      everyUnit: "minutes",
      data: "{}",
    })
  })
})

describe("validateSchedulerForm", () => {
  const valid = { ...emptySchedulerForm("Europe/Paris"), id: " nightly ", jobName: " build " }

  it("builds the scheduler to save", () => {
    expect(validateSchedulerForm(valid)).toEqual({
      ok: true,
      scheduler: {
        id: "nightly",
        jobName: "build",
        schedule: { kind: "pattern", pattern: "0 9 * * *", tz: "Europe/Paris" },
        data: "{}",
      },
    })
    expect(
      validateSchedulerForm({ ...valid, kind: "every", everyValue: "2", everyUnit: "hours" }),
    ).toMatchObject({ ok: true, scheduler: { schedule: { kind: "every", everyMs: 7_200_000 } } })
    expect(scheduleOf({ ...valid, tz: "  " })).toEqual({
      kind: "pattern",
      pattern: "0 9 * * *",
      tz: null,
    })
  })

  it("reports every problem, per field", () => {
    const result = validateSchedulerForm({
      ...valid,
      id: "a:b",
      jobName: " ",
      kind: "every",
      everyValue: "0.5",
      everyUnit: "seconds",
      data: "{oops",
    })
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(Object.keys(result.errors).sort()).toEqual(["data", "every", "id", "jobName"])
    expect(result.errors.every).toBe("At least 1 second")
  })

  it("requires a pattern in pattern mode and refuses ids with spaces", () => {
    const result = validateSchedulerForm({ ...valid, id: "my job", pattern: "  " })
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.errors.id).toMatch(/spaces/)
    expect(result.errors.pattern).toMatch(/cron pattern/)
  })
})

describe("describeSchedule", () => {
  it("reads naturally", () => {
    expect(describeSchedule({ kind: "every", everyMs: 900_000 })).toBe("every 15 minutes")
    expect(describeSchedule({ kind: "every", everyMs: 3_600_000 })).toBe("every 1 hour")
    expect(describeSchedule({ kind: "pattern", pattern: "0 9 * * *", tz: "Europe/Paris" })).toBe(
      "0 9 * * * (Europe/Paris)",
    )
    expect(describeSchedule({ kind: "pattern", pattern: "0 9 * * *", tz: null })).toBe("0 9 * * *")
  })
})
