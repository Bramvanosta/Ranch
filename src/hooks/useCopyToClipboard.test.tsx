import { act, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

const toastSuccess = vi.fn()
const toastError = vi.fn()
vi.mock("sonner", () => ({
  toast: {
    success: (msg: string, opts?: unknown) => toastSuccess(msg, opts),
    error: (msg: string, opts?: unknown) => toastError(msg, opts),
  },
}))

import { useCopyToClipboard } from "./useCopyToClipboard"

afterEach(() => {
  vi.clearAllMocks()
})

describe("useCopyToClipboard", () => {
  it("writes to navigator.clipboard and toasts on success", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.assign(navigator, { clipboard: { writeText } })

    const { result } = renderHook(() => useCopyToClipboard())
    let ok = false
    await act(async () => {
      ok = await result.current("hello", "job ID")
    })

    expect(ok).toBe(true)
    expect(writeText).toHaveBeenCalledWith("hello")
    expect(toastSuccess).toHaveBeenCalledWith(
      "Copied job ID to clipboard",
      expect.objectContaining({ duration: 2_000 }),
    )
  })

  it("renders a destructive toast and returns false when writeText rejects", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("denied"))
    Object.assign(navigator, { clipboard: { writeText } })

    const { result } = renderHook(() => useCopyToClipboard())
    let ok = true
    await act(async () => {
      ok = await result.current("hello", "payload")
    })

    expect(ok).toBe(false)
    expect(toastError).toHaveBeenCalledWith(
      "Couldn't copy payload. Try again.",
      expect.objectContaining({ duration: 3_000 }),
    )
  })
})
