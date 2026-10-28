import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    getConnectionLockState: vi.fn(),
    unlockConnection: vi.fn(),
    lockConnection: vi.fn(),
  },
}))

import { commands } from "@/lib/tauri/bindings"

import { withOneShotUnlock } from "./oneShotUnlock"

const get = commands.getConnectionLockState as unknown as ReturnType<typeof vi.fn>
const unlock = commands.unlockConnection as unknown as ReturnType<typeof vi.fn>
const lock = commands.lockConnection as unknown as ReturnType<typeof vi.fn>

afterEach(() => vi.resetAllMocks())

describe("withOneShotUnlock", () => {
  it("unlocks a locked connection for the action only, then locks it again", async () => {
    get.mockResolvedValue({ status: "ok", data: "locked" })
    unlock.mockResolvedValue({ status: "ok", data: "unlocked" })
    lock.mockResolvedValue({ status: "ok", data: "locked" })
    const order: string[] = []
    unlock.mockImplementation(async () => {
      order.push("unlock")
      return { status: "ok", data: "unlocked" }
    })
    lock.mockImplementation(async () => {
      order.push("lock")
      return { status: "ok", data: "locked" }
    })

    const result = await withOneShotUnlock("c1", async () => {
      order.push("run")
      return 7
    })
    expect(result).toBe(7)
    expect(order).toEqual(["unlock", "run", "lock"])
  })

  it("locks again when the action fails", async () => {
    get.mockResolvedValue({ status: "ok", data: "locked" })
    unlock.mockResolvedValue({ status: "ok", data: "unlocked" })
    lock.mockResolvedValue({ status: "ok", data: "locked" })
    await expect(
      withOneShotUnlock("c1", async () => {
        throw new Error("boom")
      }),
    ).rejects.toThrow("boom")
    expect(lock).toHaveBeenCalledWith({ connectionId: "c1" })
  })

  it("leaves an unlocked connection alone", async () => {
    get.mockResolvedValue({ status: "ok", data: "unlocked" })
    await withOneShotUnlock("c1", async () => undefined)
    expect(unlock).not.toHaveBeenCalled()
    expect(lock).not.toHaveBeenCalled()
  })
})
