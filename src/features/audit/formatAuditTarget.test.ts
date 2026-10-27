import { describe, expect, it } from "vitest"

import type { AuditTarget } from "@/lib/tauri/bindings"

import { formatAuditTarget } from "./formatAuditTarget"

describe("formatAuditTarget", () => {
  it("formats a job target as `queue / #id`", () => {
    const t: AuditTarget = {
      kind: "job",
      data: { queueName: "emails", jobId: "42" },
    }
    expect(formatAuditTarget(t)).toBe("emails / #42")
  })

  it("formats a queue target as the queue name", () => {
    const t: AuditTarget = { kind: "queue", data: { queueName: "emails" } }
    expect(formatAuditTarget(t)).toBe("emails")
  })

  it("formats a scheduler target as `queue / scheduler-key`", () => {
    const t: AuditTarget = {
      kind: "scheduler",
      data: { queueName: "emails", schedulerKey: "daily-report" },
    }
    expect(formatAuditTarget(t)).toBe("emails / daily-report")
  })

  it("formats a connection target as the literal `connection`", () => {
    const t: AuditTarget = { kind: "connection" }
    expect(formatAuditTarget(t)).toBe("connection")
  })
})
