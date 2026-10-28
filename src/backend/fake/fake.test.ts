import { beforeEach, describe, expect, it } from "vitest"

import { connectionsHandlers } from "./connections"
import { jobsHandlers } from "./jobs"
import { PreviewError } from "./types"
import { resetWorld } from "./world"

/**
 * The demo world the web preview and the Playwright checks run on. Its
 * handlers are plain functions over an in-memory world, so they can be
 * driven directly.
 */

const emails = "bull:emails"

beforeEach(() => resetWorld())

describe("the demo world", () => {
  it("holds Lumen Studios and its lumen-api connections", () => {
    const connections = connectionsHandlers.list_connections()
    expect(new Set(connections.map((c) => c.groupName))).toContain("Lumen Studios")
    expect(connections.filter((c) => c.name === "lumen-api").map((c) => c.envLabel)).toEqual([
      "Local",
      "Staging",
      "Production",
    ])
  })

  it("retries a failed job: it leaves Failed for Waiting", () => {
    const failed = (state: "failed" | "waiting") =>
      jobsHandlers.list_jobs({
        args: { connectionId: "c-local", queueId: emails, state, offset: 0, limit: 500 },
      })
    const before = failed("failed")
    const job = before.jobs[0]
    expect(job).toBeDefined()
    const result = jobsHandlers.retry_job({
      args: { connectionId: "c-local", queueId: emails, jobId: job.jobRef.id },
    })
    expect(result).toMatchObject({ jobId: job.jobRef.id, newState: "waiting", action: "retry" })
    expect(failed("failed").total).toBe(before.total - 1)
    expect(failed("waiting").jobs.map((j) => j.jobRef.id)).toContain(job.jobRef.id)
  })

  it("refuses a write on the locked production connection", () => {
    const write = () =>
      jobsHandlers.retry_job({ args: { connectionId: "c-prod", queueId: emails, jobId: "1" } })
    expect(write).toThrow(PreviewError)
    try {
      write()
    } catch (e) {
      expect((e as PreviewError).kind).toBe("connectionLocked")
    }
  })
})
