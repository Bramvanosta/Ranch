/**
 * One self-paced poller per query.
 */
import { QueryClient } from "@tanstack/react-query"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { resetPollsForTests, subscribePoll } from "./sharedPoll"

const KEY = ["queues", "conn-x"]

function client(readMs = 0) {
  const qc = new QueryClient()
  const reads = vi.spyOn(qc, "refetchQueries").mockImplementation(
    () =>
      new Promise((resolve) => {
        setTimeout(() => resolve(undefined), readMs)
      }),
  )
  return { qc, reads }
}

const setHidden = (hidden: boolean) => {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    get: () => (hidden ? "hidden" : "visible"),
  })
  document.dispatchEvent(new Event("visibilitychange"))
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date", "performance"] })
  setHidden(false)
})
afterEach(() => {
  resetPollsForTests()
  vi.useRealTimers()
})

describe("subscribePoll", () => {
  it("reads once per interval however many components show the query", async () => {
    const { qc, reads } = client()
    const a = subscribePoll(qc, KEY, 1_000)
    await vi.advanceTimersByTimeAsync(300)
    const b = subscribePoll(qc, KEY, 1_000)
    await vi.advanceTimersByTimeAsync(3_000)
    expect(reads).toHaveBeenCalledTimes(3)
    a()
    b()
  })

  it("follows the most eager subscriber", async () => {
    const { qc, reads } = client()
    const sidebar = subscribePoll(qc, KEY, 10_000)
    await vi.advanceTimersByTimeAsync(2_000)
    expect(reads).not.toHaveBeenCalled()
    const screen = subscribePoll(qc, KEY, 1_000)
    await vi.advanceTimersByTimeAsync(1_000)
    expect(reads).toHaveBeenCalledTimes(1)
    screen()
    sidebar()
  })

  it("spaces reads out to three times what the last one took", async () => {
    const { qc, reads } = client(2_000)
    const stop = subscribePoll(qc, KEY, 1_000)
    await vi.advanceTimersByTimeAsync(1_000) // first read starts
    await vi.advanceTimersByTimeAsync(2_000) // …and takes 2 s
    expect(reads).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(5_900)
    expect(reads).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(200)
    expect(reads).toHaveBeenCalledTimes(2)
    stop()
  })

  it("pauses while hidden and reads at once when shown", async () => {
    const { qc, reads } = client()
    const stop = subscribePoll(qc, KEY, 1_000)
    setHidden(true)
    await vi.advanceTimersByTimeAsync(5_000)
    expect(reads).not.toHaveBeenCalled()
    setHidden(false)
    await vi.advanceTimersByTimeAsync(0)
    expect(reads).toHaveBeenCalledTimes(1)
    stop()
  })

  it("stops when the last subscriber leaves", async () => {
    const { qc, reads } = client()
    const stop = subscribePoll(qc, KEY, 1_000)
    stop()
    await vi.advanceTimersByTimeAsync(5_000)
    expect(reads).not.toHaveBeenCalled()
  })
})

describe("holdsPoll", () => {
  it("skips a turn while a mutation changing the data is pending", async () => {
    const { qc, reads } = client()
    let finish: () => void = () => {}
    const pending = qc
      .getMutationCache()
      .build(qc, {
        mutationFn: () => new Promise<void>((r) => (finish = r)),
        meta: { holdsPoll: () => ["queues", "conn-x"] },
      })
      .execute(undefined)
    const stop = subscribePoll(qc, KEY, 1_000)
    await vi.advanceTimersByTimeAsync(2_500)
    expect(reads).not.toHaveBeenCalled()
    finish()
    await pending
    await vi.advanceTimersByTimeAsync(1_000)
    expect(reads).toHaveBeenCalledTimes(1)
    stop()
  })
})
