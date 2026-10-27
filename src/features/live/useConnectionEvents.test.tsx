/**
 * Live updates from BullMQ's event streams.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

const runtime = vi.hoisted(() => ({ current: "desktop" as string }))
vi.mock("@/lib/runtime/runtime", () => ({ getRuntime: () => runtime.current }))
vi.mock("@/lib/tauri/bindings", () => ({ commands: { followConnectionEvents: vi.fn() } }))
const listeners = vi.hoisted(() => new Map<string, (e: { payload: unknown }) => void>())
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    listeners.set(name, cb)
    return () => listeners.delete(name)
  }),
}))

import { commands, type QueueEventsNotice } from "@/lib/tauri/bindings"

import { QUEUE_EVENTS, refreshFor, useConnectionEvents, useIsLive } from "./useConnectionEvents"

const follow = commands.followConnectionEvents as unknown as ReturnType<typeof vi.fn>

const notice = (events: string[]): QueueEventsNotice => ({
  connectionId: "c1",
  queues: [{ queueRef: { prefix: "bull", name: "emails" }, events, count: events.length }],
})

const setHidden = (hidden: boolean) => {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    get: () => (hidden ? "hidden" : "visible"),
  })
  document.dispatchEvent(new Event("visibilitychange"))
}

afterEach(() => {
  vi.clearAllMocks()
  runtime.current = "desktop"
  setHidden(false)
})

describe("refreshFor", () => {
  it("reads again the counters and the lists of the queues that moved", () => {
    const qc = new QueryClient()
    const spy = vi.spyOn(qc, "invalidateQueries")
    refreshFor(qc, notice(["added", "completed"]))
    const keys = spy.mock.calls.map((c) => JSON.stringify(c[0]?.queryKey))
    expect(keys).toContain(JSON.stringify(["queues", "c1"]))
    expect(keys).toContain(JSON.stringify(["jobs", "c1", "bull", "emails"]))
    expect(keys).toContain(JSON.stringify(["job", "c1", "bull", "emails"]))
    expect(keys).not.toContain(JSON.stringify(["failed-jobs", "c1"]))
  })

  it("reads the failed views again when something failed", () => {
    const qc = new QueryClient()
    const spy = vi.spyOn(qc, "invalidateQueries")
    refreshFor(qc, notice(["failed"]))
    const keys = spy.mock.calls.map((c) => JSON.stringify(c[0]?.queryKey))
    expect(keys).toContain(JSON.stringify(["failed-jobs", "c1"]))
    expect(keys).toContain(JSON.stringify(["failureCauses", "c1"]))
  })
})

describe("useConnectionEvents", () => {
  const wrapper = () => {
    const qc = new QueryClient()
    return {
      qc,
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={qc}>{children}</QueryClientProvider>
      ),
    }
  }

  it("follows while shown, stops while hidden and when left", async () => {
    follow.mockResolvedValue({ status: "ok", data: true })
    const { wrapper: w } = wrapper()
    const { result, unmount } = renderHook(
      () => {
        useConnectionEvents("c1")
        return useIsLive("c1")
      },
      { wrapper: w },
    )
    await waitFor(() => expect(result.current).toBe(true))
    expect(follow).toHaveBeenLastCalledWith({ connectionId: "c1", follow: true })

    act(() => setHidden(true))
    await waitFor(() => expect(result.current).toBe(false))
    expect(follow).toHaveBeenLastCalledWith({ connectionId: "c1", follow: false })

    act(() => setHidden(false))
    await waitFor(() => expect(result.current).toBe(true))
    unmount()
    expect(follow).toHaveBeenLastCalledWith({ connectionId: "c1", follow: false })
  })

  it("refreshes on this connection's notices only", async () => {
    follow.mockResolvedValue({ status: "ok", data: true })
    const { qc, wrapper: w } = wrapper()
    const spy = vi.spyOn(qc, "invalidateQueries")
    const { unmount } = renderHook(() => useConnectionEvents("c1"), { wrapper: w })
    await waitFor(() => expect(listeners.has(QUEUE_EVENTS)).toBe(true))
    listeners.get(QUEUE_EVENTS)?.({ payload: { ...notice(["added"]), connectionId: "c2" } })
    expect(spy).not.toHaveBeenCalled()
    listeners.get(QUEUE_EVENTS)?.({ payload: notice(["added"]) })
    expect(spy).toHaveBeenCalled()
    unmount()
  })

  it("does nothing outside the desktop app (the server and the preview poll)", () => {
    runtime.current = "server"
    const { wrapper: w } = wrapper()
    renderHook(() => useConnectionEvents("c1"), { wrapper: w })
    expect(follow).not.toHaveBeenCalled()
  })
})
