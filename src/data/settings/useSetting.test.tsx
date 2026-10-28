import { act, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { getKvSetting: vi.fn(), setKvSetting: vi.fn() },
}))
vi.mock("@/lib/settingsSync", () => ({ announceSettingsChange: vi.fn() }))
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }))

import { toast } from "sonner"
import { announceSettingsChange } from "@/lib/settingsSync"
import { commands } from "@/lib/tauri/bindings"
import { renderHookWithProviders } from "@/test/renderWithProviders"
import { err, ok } from "@/test/results"

import { jobPageSize } from "./settings"
import { readSettingOrDefault, settingQueryKey, useSetting } from "./useSetting"

afterEach(() => vi.resetAllMocks())

describe("useSetting", () => {
  it("reads the stored value, under the kv query key", async () => {
    vi.mocked(commands.getKvSetting).mockResolvedValue(ok("100"))
    const { result, client } = renderHookWithProviders(() => useSetting(jobPageSize))
    expect(result.current.value).toBe(50)
    expect(result.current.stored).toBeUndefined()
    await waitFor(() => expect(result.current.stored).toBe(100))
    expect(commands.getKvSetting).toHaveBeenCalledWith("jobs.pageSize")
    expect(client.getQueryData(settingQueryKey(jobPageSize))).toBe(100)
  })

  it("shows the default when the value cannot be read", async () => {
    vi.mocked(commands.getKvSetting).mockResolvedValue(err("database"))
    const { result } = renderHookWithProviders(() => useSetting(jobPageSize))
    await waitFor(() => expect(result.current.query.isError).toBe(true))
    expect(result.current.value).toBe(50)
  })

  it("applies a new value at once, stores it, and tells the other windows", async () => {
    vi.mocked(commands.getKvSetting).mockResolvedValue(ok(null))
    vi.mocked(commands.setKvSetting).mockResolvedValue(ok(null))
    const { result, client } = renderHookWithProviders(() => useSetting(jobPageSize))
    await waitFor(() => expect(result.current.stored).toBe(50))
    act(() => result.current.set(250))
    expect(client.getQueryData(settingQueryKey(jobPageSize))).toBe(250)
    await waitFor(() => expect(result.current.value).toBe(250))
    await waitFor(() => expect(announceSettingsChange).toHaveBeenCalledWith({ topic: "kv" }))
    expect(commands.setKvSetting).toHaveBeenCalledWith("jobs.pageSize", "250")
  })

  it("says so when a value cannot be stored", async () => {
    vi.mocked(commands.getKvSetting).mockResolvedValue(ok(null))
    vi.mocked(commands.setKvSetting).mockResolvedValue(err("database", "disk full"))
    const { result } = renderHookWithProviders(() => useSetting(jobPageSize))
    await waitFor(() => expect(result.current.stored).toBe(50))
    act(() => result.current.set(25))
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("Could not save: disk full"))
    expect(announceSettingsChange).not.toHaveBeenCalled()
  })
})

describe("readSettingOrDefault", () => {
  it("falls back to the default instead of failing", async () => {
    vi.mocked(commands.getKvSetting).mockResolvedValue(err("database"))
    await expect(readSettingOrDefault(jobPageSize)).resolves.toBe(50)
  })
})
