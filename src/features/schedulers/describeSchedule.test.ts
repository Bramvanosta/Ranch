import { describe, expect, it } from "vitest"

import type { Job } from "@/lib/tauri/bindings"

import { describeCron, describeSchedule, runsOf } from "./describeSchedule"

describe("describeCron", () => {
  it("says the common shapes in plain language", () => {
    const cases: Array<[string, string | null]> = [
      ["* * * * *", "every minute"],
      ["*/5 * * * *", "every 5 minutes"],
      ["7 * * * *", "hourly at :07"],
      ["0 */6 * * *", "every 6 hours at :00"],
      ["0 9 * * *", "daily at 09:00"],
      ["30 8 * * 1-5", "weekdays at 08:30"],
      ["0 18 * * 5", "every Friday at 18:00"],
      ["0 3 1 * *", "monthly on day 1 at 03:00"],
      ["0 0 9 * * *", "daily at 09:00"],
      ["15 0 9 * * *", null],
      ["0 9 * 1 *", null],
      ["0 9,17 * * *", null],
    ]
    for (const [pattern, want] of cases) expect(describeCron(pattern), pattern).toBe(want)
  })

  it("an interval is said as one; a zone is named", () => {
    expect(describeSchedule({ pattern: null, every: 300_000, tz: null }).main).toBe("every 5m")
    expect(describeSchedule({ pattern: "0 9 * * *", every: null, tz: "Europe/Paris" })).toEqual({
      main: "0 9 * * *",
      plain: "daily at 09:00 (Europe/Paris)",
    })
  })
})

describe("runsOf", () => {
  const job = (id: string, name: string, finishedOn: number) =>
    ({
      jobRef: { queueId: "bull:q", queueName: "q", id },
      name,
      finishedOn,
      timestamp: 0,
      processedOn: null,
    }) as unknown as Job

  it("prefers the scheduler's own job ids, newest first", () => {
    const jobs = [
      job("repeat:digest:1", "send", 10),
      job("9", "send", 50),
      job("repeat:digest:2", "send", 20),
    ]
    expect(runsOf({ key: "digest", name: "send" }, jobs).map((j) => j.jobRef.id)).toEqual([
      "repeat:digest:2",
      "repeat:digest:1",
    ])
  })

  it("falls back on the job name", () => {
    expect(runsOf({ key: "digest", name: "send" }, [job("9", "send", 50)])).toHaveLength(1)
  })
})
