import { waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { getJob: vi.fn() },
}))

import { useJob } from "@/data/jobs/queries"
import { commands } from "@/lib/tauri/bindings"
import { aJob, aQueueId } from "@/test/factories"
import { renderHookWithProviders } from "@/test/renderWithProviders"
import { err, ok } from "@/test/results"

import { errorKind, okOr, RanchError, unwrap } from "./result"

afterEach(() => {
  vi.resetAllMocks()
})

describe("unwrap", () => {
  it("returns the data of an ok result", () => {
    expect(unwrap(ok([1, 2]))).toEqual([1, 2])
  })

  it("throws a RanchError that keeps the kind, the message and the help link", () => {
    const failure = {
      status: "error" as const,
      error: {
        kind: "keychainUnavailable",
        message: "The keychain is locked",
        docUrl: "https://example.test/keychain",
      },
    }
    let thrown: unknown
    try {
      unwrap(failure)
    } catch (e) {
      thrown = e
    }
    expect(thrown).toBeInstanceOf(RanchError)
    expect(thrown).toBeInstanceOf(Error)
    expect(thrown).toMatchObject({
      kind: "keychainUnavailable",
      message: "The keychain is locked",
      docUrl: "https://example.test/keychain",
    })
  })
})

describe("okOr", () => {
  it("gives the data of an ok result and the fallback of an error", () => {
    expect(okOr(ok("1"), null)).toBe("1")
    expect(okOr(err("redis"), null)).toBeNull()
  })
})

describe("errorKind", () => {
  it("reads the kind of a backend failure, and of nothing else", () => {
    expect(errorKind(new RanchError({ kind: "jobNotFound", message: "gone", docUrl: null }))).toBe(
      "jobNotFound",
    )
    expect(errorKind({ kind: "connectionLocked" })).toBe("connectionLocked")
    expect(errorKind(new Error("network down"))).toBeUndefined()
    expect(errorKind(null)).toBeUndefined()
    expect(errorKind({ kind: 42 })).toBeUndefined()
  })
})

describe("a backend failure on screen", () => {
  it("reaches the query's error with its kind, so the screen can say the job is gone", async () => {
    vi.mocked(commands.getJob).mockResolvedValue(err("jobNotFound", "Job 7 not found"))
    const { result } = renderHookWithProviders(() =>
      useJob({ connectionId: "c1", queueId: aQueueId(), jobId: "7" }),
    )
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(result.current.error).toBeInstanceOf(RanchError)
    expect(errorKind(result.current.error)).toBe("jobNotFound")
    expect(result.current.error?.message).toBe("Job 7 not found")
  })

  it("leaves a successful read untouched", async () => {
    vi.mocked(commands.getJob).mockResolvedValue(ok(aJob({ id: "7" })))
    const { result } = renderHookWithProviders(() =>
      useJob({ connectionId: "c1", queueId: aQueueId(), jobId: "7" }),
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.jobRef.id).toBe("7")
  })
})
