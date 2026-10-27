import { QueryClient } from "@tanstack/react-query"
import { describe, expect, it, vi } from "vitest"

import { applySettingsChange } from "./settingsSync"

describe("applySettingsChange", () => {
  it("switches the theme of this window", () => {
    const setTheme = vi.fn()
    applySettingsChange({ topic: "theme", theme: "dark" }, new QueryClient(), setTheme)
    expect(setTheme).toHaveBeenCalledWith("dark")
  })

  it("re-reads the kv settings, and only them, on a kv change", () => {
    const client = new QueryClient()
    const invalidate = vi.spyOn(client, "invalidateQueries")
    applySettingsChange({ topic: "kv" }, client, vi.fn())
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["kv"] })
    expect(invalidate).toHaveBeenCalledTimes(1)
  })

  it("re-reads the agent safety settings and the connections on an mcp change", () => {
    const client = new QueryClient()
    const invalidate = vi.spyOn(client, "invalidateQueries")
    applySettingsChange({ topic: "mcp" }, client, vi.fn())
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["mcpSafetySettings"] })
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["connections"] })
  })
})
