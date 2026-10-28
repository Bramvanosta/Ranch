/**
 * Appearance and refresh settings: the theme applies at
 * once, the polled hooks obey the refresh setting, and every connection view
 * has a manual refresh that says when auto-refresh is off.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

const setTheme = vi.fn()
let currentTheme = "system"
vi.mock("next-themes", () => ({
  useTheme: () => ({ theme: currentTheme, setTheme }),
}))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    getKvSetting: vi.fn(),
    setKvSetting: vi.fn(),
    listQueues: vi.fn(),
  },
}))

import { useQueues } from "@/data/queues/queries"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { commands } from "@/lib/tauri/bindings"
import { AppearanceSection } from "./AppearanceSection"
import { RefreshControl } from "./RefreshControl"
import { RefreshSection } from "./RefreshSection"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>

function stored(values: Record<string, string | null>) {
  mocked.getKvSetting.mockImplementation(async (key: string) => ({
    status: "ok",
    data: values[key] ?? null,
  }))
}

function makeWrapper() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  return { client, Wrapper }
}

beforeEach(() => {
  currentTheme = "system"
  mocked.setKvSetting.mockResolvedValue({ status: "ok", data: null })
  mocked.listQueues.mockResolvedValue({ status: "ok", data: [] })
})

afterEach(() => {
  vi.resetAllMocks()
})

describe("AppearanceSection", () => {
  it("shows the current theme and applies another at once", () => {
    currentTheme = "dark"
    render(<AppearanceSection />)
    expect(screen.getByTestId("theme-dark")).toBeChecked()
    expect(screen.getByTestId("theme-system")).not.toBeChecked()
    fireEvent.click(screen.getByTestId("theme-light"))
    expect(setTheme).toHaveBeenCalledWith("light")
  })
})

describe("RefreshSection", () => {
  it("loads the stored setting and saves a change", async () => {
    stored({ "refresh.auto": "1", "refresh.intervalMs": "5000" })
    const { Wrapper } = makeWrapper()
    render(<RefreshSection />, { wrapper: Wrapper })
    const interval = screen.getByTestId("refresh-interval") as HTMLSelectElement
    await waitFor(() => expect(interval.value).toBe("5000"))
    expect(Array.from(interval.options).map((o) => o.text)).toEqual([
      "1 s",
      "2 s",
      "5 s",
      "10 s",
      "30 s",
    ])

    fireEvent.change(interval, { target: { value: "30000" } })
    await waitFor(() =>
      expect(mocked.setKvSetting).toHaveBeenCalledWith("refresh.intervalMs", "30000"),
    )
    expect(interval.value).toBe("30000")

    fireEvent.click(screen.getByTestId("refresh-auto"))
    await waitFor(() => expect(mocked.setKvSetting).toHaveBeenCalledWith("refresh.auto", "0"))
    expect(interval).toBeDisabled()
  })
})

describe("useRefetchInterval", () => {
  it("is the user's interval, never below the view's floor, and false when off", async () => {
    stored({ "refresh.auto": "1", "refresh.intervalMs": "2000" })
    const a = makeWrapper()
    const counters = renderHook(() => useRefetchInterval(1_000), { wrapper: a.Wrapper })
    const dashboard = renderHook(() => useRefetchInterval(5_000), { wrapper: a.Wrapper })
    await waitFor(() => expect(counters.result.current).toBe(2_000))
    expect(dashboard.result.current).toBe(5_000)

    stored({ "refresh.auto": "0", "refresh.intervalMs": "2000" })
    const b = makeWrapper()
    const off = renderHook(() => useRefetchInterval(1_000), { wrapper: b.Wrapper })
    await waitFor(() => expect(off.result.current).toBe(false))
  })

  it("keeps Ranch's historical 1 s when the setting cannot be read", async () => {
    mocked.getKvSetting.mockRejectedValue(new Error("db locked"))
    const { Wrapper } = makeWrapper()
    const hook = renderHook(() => useRefetchInterval(1_000), { wrapper: Wrapper })
    await new Promise((r) => setTimeout(r, 20))
    expect(hook.result.current).toBe(1_000)
  })
})

describe("polling obeys the setting", () => {
  it("does not poll the queue counters with auto-refresh off", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    stored({ "refresh.auto": "0" })
    const { Wrapper } = makeWrapper()
    renderHook(() => useQueues("c1"), { wrapper: Wrapper })
    await waitFor(() => expect(mocked.listQueues).toHaveBeenCalledTimes(1))
    await vi.advanceTimersByTimeAsync(5_000)
    expect(mocked.listQueues).toHaveBeenCalledTimes(1)
    vi.useRealTimers()
  })
})

describe("RefreshControl", () => {
  it("refetches what belongs to the connection, and only that", async () => {
    stored({})
    const { client, Wrapper } = makeWrapper()
    const invalidate = vi.spyOn(client, "invalidateQueries")
    render(<RefreshControl connectionId="c1" />, { wrapper: Wrapper })
    expect(screen.queryByTestId("auto-refresh-off")).not.toBeInTheDocument()

    fireEvent.click(screen.getByTestId("refresh-now"))
    expect(invalidate).toHaveBeenCalledTimes(1)
    const { predicate } = invalidate.mock.calls[0][0] as unknown as {
      predicate: (q: { queryKey: unknown[] }) => boolean
    }
    expect(predicate({ queryKey: ["queues", "c1"] })).toBe(true)
    expect(predicate({ queryKey: ["jobs", "c1", "bull:emails", "failed", 0] })).toBe(true)
    expect(predicate({ queryKey: ["queues", "c2"] })).toBe(false)
    expect(predicate({ queryKey: ["kv", "refresh"] })).toBe(false)
  })

  it("turns for a click, not for the background polling", async () => {
    stored({})
    const { client, Wrapper } = makeWrapper()
    let release: (queues: never[]) => void = () => undefined
    mocked.listQueues.mockImplementation(
      () => new Promise((resolve) => (release = resolve as typeof release)),
    )
    renderHook(() => useQueues("c1"), { wrapper: Wrapper })
    render(<RefreshControl connectionId="c1" />, { wrapper: Wrapper })
    const icon = () => screen.getByTestId("refresh-now").querySelector("svg")

    // A query of the connection is in flight (the poll): the icon stays still.
    await waitFor(() => expect(client.isFetching()).toBeGreaterThan(0))
    expect(icon()).toHaveAttribute("data-spinning", "false")
    release([])
    await waitFor(() => expect(client.isFetching()).toBe(0))

    // A click: it turns until the refetch is back.
    fireEvent.click(screen.getByTestId("refresh-now"))
    await waitFor(() => expect(icon()).toHaveAttribute("data-spinning", "true"))
    release([])
    await waitFor(() => expect(icon()).toHaveAttribute("data-spinning", "false"))
  })

  it("says when auto-refresh is off", async () => {
    stored({ "refresh.auto": "0" })
    const { Wrapper } = makeWrapper()
    render(<RefreshControl connectionId="c1" />, { wrapper: Wrapper })
    expect(await screen.findByTestId("auto-refresh-off")).toHaveTextContent("auto-refresh off")
  })
})
