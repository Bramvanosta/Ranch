import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({ commands: { openWindow: vi.fn() } }))

import { setRuntime } from "@/lib/runtime/runtime"
import { commands } from "@/lib/tauri/bindings"

import { isMainWindow, jobRoute, openRoute, queueRoute } from "./openRoute"

const openWindow = commands.openWindow as unknown as ReturnType<typeof vi.fn>
const internals = (label?: string) => {
  ;(window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = label
    ? { metadata: { currentWindow: { label } } }
    : undefined
}

describe("windows and tabs", () => {
  afterEach(() => {
    setRuntime("desktop")
    internals()
    vi.restoreAllMocks()
  })

  it("the desktop app opens a native window or tab at the route", async () => {
    setRuntime("desktop")
    openWindow.mockResolvedValue({ status: "ok", data: null })
    await openRoute("/connections/c1/queues/emails?view=jobs", { tab: true })
    expect(openWindow).toHaveBeenCalledWith({
      route: "/connections/c1/queues/emails?view=jobs",
      tab: true,
    })
  })

  it("a browser opens a browser tab", async () => {
    setRuntime("server")
    const open = vi.spyOn(window, "open").mockReturnValue(null)
    await openRoute("/audit", { tab: false })
    expect(open).toHaveBeenCalledWith("/audit", "_blank", "noopener")
  })

  it("only the main window remembers the context for the next launch", () => {
    setRuntime("desktop")
    internals("main")
    expect(isMainWindow()).toBe(true)
    internals("win-1793")
    expect(isMainWindow()).toBe(false)
    setRuntime("server")
    expect(isMainWindow()).toBe(true)
  })

  it("routes are built with every part encoded", () => {
    expect(queueRoute("c1", "a b")).toBe("/connections/c1/queues/a%20b")
    expect(jobRoute("c1", "emails", "#enrich/1")).toBe(
      "/connections/c1/queues/emails/%23enrich%2F1",
    )
  })
})
