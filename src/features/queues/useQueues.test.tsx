import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listQueues: vi.fn() },
}))

import { commands } from "@/lib/tauri/bindings"

import { useQueues } from "./useQueues"

const mocked = commands as unknown as { listQueues: ReturnType<typeof vi.fn> }

afterEach(() => {
  vi.resetAllMocks()
  vi.useRealTimers()
})

function wrapper({ retry = false }: { retry?: boolean | ((n: number, e: Error) => boolean) } = {}) {
  // Each test gets its own client so retries / cache don't bleed between tests.
  const client = new QueryClient({
    defaultOptions: { queries: { retry } },
  })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

describe("useQueues", () => {
  it("returns the data on a successful list_queues", async () => {
    mocked.listQueues.mockResolvedValue({
      status: "ok",
      data: [
        {
          queueRef: { prefix: "bull", name: "emails" },
          counters: {
            waiting: 5,
            active: 0,
            completed: 100,
            failed: 1,
            delayed: 0,
            prioritized: 0,
            paused: 0,
          },
          isPaused: false,
        },
      ],
    })
    const { result } = renderHook(() => useQueues("conn-x"), { wrapper: wrapper() })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data).toHaveLength(1)
    expect(result.current.data?.[0].queueRef.name).toBe("emails")
  })

  it("does NOT retry on schemaNotRecognized — fails immediately", async () => {
    mocked.listQueues.mockResolvedValue({
      status: "error",
      error: {
        kind: "schemaNotRecognized",
        message: "BullMQ schema not recognized (detected version: 7.0.0)",
        docUrl: null,
      },
    })
    const { result } = renderHook(() => useQueues("conn-bad"), { wrapper: wrapper() })
    await waitFor(() => expect(result.current.isError).toBe(true))
    // mocked.listQueues called exactly once — no retry.
    expect(mocked.listQueues).toHaveBeenCalledTimes(1)
    expect((result.current.error as Error).message).toContain("not recognized")
  })

  it("polls at 1s intervals while the document is visible", async () => {
    vi.useFakeTimers()
    mocked.listQueues.mockResolvedValue({ status: "ok", data: [] })
    const { unmount } = renderHook(() => useQueues("conn-poll"), { wrapper: wrapper() })

    // Wait for the initial fetch to settle.
    await vi.advanceTimersByTimeAsync(50)
    expect(mocked.listQueues).toHaveBeenCalledTimes(1)

    // Advance 1 second → polling kicks in.
    await vi.advanceTimersByTimeAsync(1_050)
    expect(mocked.listQueues).toHaveBeenCalledTimes(2)

    // Another tick.
    await vi.advanceTimersByTimeAsync(1_050)
    expect(mocked.listQueues).toHaveBeenCalledTimes(3)

    unmount()
  })
})
