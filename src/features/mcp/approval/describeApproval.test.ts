import { describe, expect, it } from "vitest"
import { BULLMQ_CAPABILITIES } from "@/backend/fake/backends"

import type { PendingApproval } from "@/lib/tauri/bindings"

import {
  describeMaintenanceScope,
  describeRequest,
  describeSchedulerTiming,
  isDestructive,
  parsePreview,
  secondsLeft,
} from "./describeApproval"

const approval = (over: Partial<PendingApproval>): PendingApproval => ({
  id: "a1",
  createdAt: "2026-09-18T10:00:00Z",
  clientHint: "Claude Desktop",
  connectionId: "c1",
  connectionName: "queues",
  groupName: "Lumen",
  environmentName: "Staging",
  envColor: "env-staging",
  tool: "remove_job",
  kind: "write",
  argumentsJson: "{}",
  targetPreviewJson: null,
  timeoutSeconds: 120,
  ...over,
})

describe("describeRequest", () => {
  it("names the client, the verb and the object", () => {
    expect(describeRequest(approval({}))).toBe("Claude Desktop wants to remove a job")
    expect(describeRequest(approval({ tool: "pause_queue" }))).toBe(
      "Claude Desktop wants to pause a queue",
    )
  })

  it("uses the affected count for bulk tools", () => {
    const bulk = JSON.stringify({ kind: "bulk", count: 12, sampleNames: ["a"] })
    expect(describeRequest(approval({ tool: "bulk_remove_jobs", targetPreviewJson: bulk }))).toBe(
      "Claude Desktop wants to remove 12 jobs",
    )
    const one = JSON.stringify({ kind: "bulk", count: 1 })
    expect(describeRequest(approval({ tool: "bulk_retry_jobs", targetPreviewJson: one }))).toBe(
      "Claude Desktop wants to retry 1 job",
    )
  })

  it("reads the machine identifier as a name", () => {
    expect(describeRequest(approval({ clientHint: "local-agent-mode-ranch" }))).toBe(
      "Claude (agent mode) wants to remove a job",
    )
    expect(describeRequest(approval({ clientHint: "some-other-agent" }))).toBe(
      "Some other agent wants to remove a job",
    )
  })

  it("falls back gracefully for an unknown client or tool", () => {
    expect(describeRequest(approval({ clientHint: null }))).toBe("An agent wants to remove a job")
    expect(describeRequest(approval({ tool: "future_tool" }))).toBe(
      "Claude Desktop wants to run future_tool",
    )
    expect(describeRequest(approval({ tool: "list_jobs", kind: "read" }))).toBe(
      "Claude Desktop wants to read with list_jobs",
    )
  })
})

describe("parsePreview", () => {
  it("recognises job and bulk previews, keeps anything else as raw data", () => {
    expect(parsePreview(JSON.stringify({ kind: "job", name: "send" }))).toMatchObject({
      kind: "job",
      name: "send",
    })
    expect(parsePreview(JSON.stringify({ kind: "bulk", count: 3 }))).toMatchObject({ count: 3 })
    expect(
      parsePreview(JSON.stringify({ kind: "newJob", queue: "emails", name: "send", data: [1] })),
    ).toEqual({ kind: "newJob", queue: "emails", name: "send", data: [1] })
    expect(parsePreview(JSON.stringify({ pattern: "* * * * *" }))).toEqual({
      kind: "raw",
      value: { pattern: "* * * * *" },
    })
    expect(parsePreview("not json")).toBeNull()
    expect(parsePreview(null)).toBeNull()
  })
})

describe("isDestructive / secondsLeft", () => {
  it("flags tools that lose data", () => {
    expect(isDestructive("remove_job")).toBe(true)
    expect(isDestructive("drain_queue")).toBe(true)
    expect(isDestructive("retry_job")).toBe(false)
    expect(isDestructive("future_tool")).toBe(false)
  })

  it("counts down from created_at and never goes negative", () => {
    const a = approval({})
    const t0 = Date.parse(a.createdAt)
    expect(secondsLeft(a, t0)).toBe(120)
    expect(secondsLeft(a, t0 + 45_000)).toBe(75)
    expect(secondsLeft(a, t0 + 500_000)).toBe(0)
  })
})

describe("maintenance previews", () => {
  it("is recognised only with a count", () => {
    expect(
      parsePreview(JSON.stringify({ kind: "maintenance", queue: "emails", count: 1200 })),
    ).toMatchObject({ kind: "maintenance", count: 1200 })
    expect(parsePreview(JSON.stringify({ kind: "maintenance", queue: "emails" }))).toMatchObject({
      kind: "raw",
    })
  })

  it("says in words what the operation targets", () => {
    const scope = (operation: unknown) =>
      describeMaintenanceScope(
        { kind: "maintenance", count: 1, operation } as never,
        BULLMQ_CAPABILITIES.jobStates,
      )
    expect(scope({ kind: "drain", includeDelayed: true })).toBe(
      "every waiting, paused, prioritized and delayed job",
    )
    expect(scope({ kind: "drain", includeDelayed: false })).toBe(
      "every waiting, paused and prioritized job",
    )
    expect(scope({ kind: "clean", state: "completed", olderThanMs: 0 })).toBe("every completed job")
    expect(scope({ kind: "clean", state: "failed", olderThanMs: 7 * 86_400_000 })).toBe(
      "failed jobs that finished more than 7 days ago",
    )
    expect(scope({ kind: "clean", state: "failed", olderThanMs: 3_600_000 })).toBe(
      "failed jobs that finished more than 1 hour ago",
    )
    expect(scope({ kind: "retryAllFailed" })).toBe("every failed job")
  })
})

describe("scheduler previews", () => {
  it("recognises a change and a removal", () => {
    expect(
      parsePreview(JSON.stringify({ kind: "schedulerChange", schedulerId: "n" })),
    ).toMatchObject({ kind: "schedulerChange", schedulerId: "n" })
    expect(
      parsePreview(JSON.stringify({ kind: "schedulerRemoval", schedulerId: "n" })),
    ).toMatchObject({ kind: "schedulerRemoval" })
  })

  it("describes the timing of what is asked for, or of what exists", () => {
    expect(
      describeSchedulerTiming({
        kind: "schedulerChange",
        schedule: { kind: "pattern", pattern: "0 3 * * *", tz: "Europe/Paris" },
      }),
    ).toBe("0 3 * * * (Europe/Paris)")
    expect(
      describeSchedulerTiming({
        kind: "schedulerChange",
        schedule: { kind: "every", everyMs: 900_000 },
      }),
    ).toBe("every 900,000 ms")
    expect(
      describeSchedulerTiming({ kind: "schedulerRemoval", pattern: "*/5 * * * *", tz: null }),
    ).toBe("*/5 * * * *")
    expect(describeSchedulerTiming({ kind: "schedulerRemoval" })).toBeNull()
  })
})
