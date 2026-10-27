import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    getKvSetting: vi.fn(),
    setKvSetting: vi.fn(),
  },
}))

import { commands } from "@/lib/tauri/bindings"

import { useDismissedHint } from "./useDismissedHint"

const mockedGet = commands.getKvSetting as unknown as ReturnType<typeof vi.fn>
const mockedSet = commands.setKvSetting as unknown as ReturnType<typeof vi.fn>

afterEach(() => {
  vi.resetAllMocks()
})

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

describe("useDismissedHint", () => {
  it("reads `dismissed=true` when kv stores '1'", async () => {
    mockedGet.mockResolvedValue({ status: "ok", data: "1" })
    const { result } = renderHook(() => useDismissedHint("cross-queue-scope"), {
      wrapper: wrap(),
    })
    await waitFor(() => expect(result.current.dismissed).toBe(true))
    expect(mockedGet).toHaveBeenCalledWith("hint:cross-queue-scope:dismissed")
  })

  it("reads `dismissed=false` when kv returns null", async () => {
    mockedGet.mockResolvedValue({ status: "ok", data: null })
    const { result } = renderHook(() => useDismissedHint("cross-queue-scope"), {
      wrapper: wrap(),
    })
    await waitFor(() => expect(result.current.dismissed).toBe(false))
  })

  it("dismiss() writes '1' and updates the cached value", async () => {
    mockedGet.mockResolvedValue({ status: "ok", data: null })
    mockedSet.mockResolvedValue({ status: "ok", data: null })
    const { result } = renderHook(() => useDismissedHint("cross-queue-scope"), {
      wrapper: wrap(),
    })
    await waitFor(() => expect(result.current.dismissed).toBe(false))
    await act(async () => {
      result.current.dismiss()
    })
    await waitFor(() => expect(result.current.dismissed).toBe(true))
    expect(mockedSet).toHaveBeenCalledWith("hint:cross-queue-scope:dismissed", "1")
  })
})
