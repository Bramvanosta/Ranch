import { listen } from "@tauri-apps/api/event"
import { afterEach, describe, expect, it, vi } from "vitest"

import { emitEvent, installTauriShim } from "./tauriShim"

afterEach(() => {
  const target = window as unknown as Record<string, unknown>
  delete target.__TAURI_INTERNALS__
  delete target.__TAURI_EVENT_PLUGIN_INTERNALS__
})

describe("installTauriShim", () => {
  it("delivers what the page emits to its listeners, until they stop listening", async () => {
    installTauriShim(vi.fn())
    const seen: unknown[] = []
    const unlisten = await listen("updater:progress", (e) => seen.push(e.payload))
    const other = vi.fn()
    await listen("bulk:progress", other)

    emitEvent("updater:progress", { phase: "verifying" })
    expect(seen).toEqual([{ phase: "verifying" }])
    expect(other).not.toHaveBeenCalled()

    unlisten()
    emitEvent("updater:progress", { phase: "installing" })
    expect(seen).toHaveLength(1)
  })

  it("sends app commands to the backend it is given", async () => {
    const invoke = vi.fn(async (_command: string, _args: Record<string, unknown>) => "pong")
    installTauriShim(invoke)
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: typeof invoke } })
      .__TAURI_INTERNALS__
    await expect(internals.invoke("ping_connection", { id: "c" })).resolves.toBe("pong")
    expect(invoke).toHaveBeenCalledWith("ping_connection", { id: "c" })
  })
})
