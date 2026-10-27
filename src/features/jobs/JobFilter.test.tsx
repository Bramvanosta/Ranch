import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({
  toast: vi.fn(),
}))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    getKvSetting: vi.fn().mockResolvedValue({ status: "ok", data: "1" }),
    setKvSetting: vi.fn().mockResolvedValue({ status: "ok", data: null }),
  },
}))

import { JobFilter } from "./JobFilter"

afterEach(() => {
  vi.resetAllMocks()
})

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

const baseProps = {
  filter: { name: "", error: "" },
  onFilterChange: vi.fn(),
  scope: "queue" as const,
  onScopeChange: vi.fn(),
  totalCount: 50,
  filteredCount: 50,
}

describe("JobFilter", () => {
  it("typing in the name segment fires onFilterChange with the new value", () => {
    const onFilterChange = vi.fn()
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobFilter {...baseProps} onFilterChange={onFilterChange} />
      </Wrapper>,
    )
    fireEvent.change(screen.getByTestId("job-filter-name"), { target: { value: "send" } })
    expect(onFilterChange).toHaveBeenCalledWith({ name: "send", error: "" })
  })

  it("typing in the error segment fires onFilterChange", () => {
    const onFilterChange = vi.fn()
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobFilter {...baseProps} onFilterChange={onFilterChange} />
      </Wrapper>,
    )
    fireEvent.change(screen.getByTestId("job-filter-error"), {
      target: { value: "Connection" },
    })
    expect(onFilterChange).toHaveBeenCalledWith({ name: "", error: "Connection" })
  })

  it("both scopes are on screen: clicking the other one switches", () => {
    const onScopeChange = vi.fn()
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobFilter {...baseProps} onScopeChange={onScopeChange} />
      </Wrapper>,
    )
    expect(screen.getByTestId("job-filter-scope-queue")).toHaveAttribute("aria-pressed", "true")
    expect(screen.getByTestId("job-filter-scope-cross")).toHaveAttribute("aria-pressed", "false")
    fireEvent.click(screen.getByTestId("job-filter-scope-cross"))
    expect(onScopeChange).toHaveBeenCalledWith("cross")
  })

  it("cross scope: data-scope=cross and the pressed button follows", () => {
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobFilter {...baseProps} scope="cross" />
      </Wrapper>,
    )
    expect(screen.getByTestId("job-filter-scope")).toHaveAttribute("data-scope", "cross")
    expect(screen.getByTestId("job-filter-scope-cross")).toHaveAttribute("aria-pressed", "true")
  })

  it("offers the error filter only where a list can hold errors — or while it still filters", () => {
    const Wrapper = wrap()
    const { rerender } = render(
      <Wrapper>
        <JobFilter {...baseProps} showErrorFilter={false} />
      </Wrapper>,
    )
    expect(screen.queryByTestId("job-filter-error")).not.toBeInTheDocument()
    rerender(
      <Wrapper>
        <JobFilter {...baseProps} showErrorFilter={false} filter={{ name: "", error: "503" }} />
      </Wrapper>,
    )
    expect(screen.getByTestId("job-filter-error")).toHaveValue("503")
  })

  it("match counter shows just the total when no filter is active", () => {
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobFilter {...baseProps} totalCount={100} filteredCount={100} />
      </Wrapper>,
    )
    expect(screen.getByTestId("job-filter-counter")).toHaveTextContent("100 jobs")
  })

  it("match counter shows `{filtered} of {total}` when a filter is active", () => {
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobFilter
          {...baseProps}
          filter={{ name: "send", error: "" }}
          totalCount={100}
          filteredCount={3}
        />
      </Wrapper>,
    )
    expect(screen.getByTestId("job-filter-counter")).toHaveTextContent("3 of 100 jobs")
  })

  it("shows the empty-result helper with `Clear filter` + `Expand to all queues` when filter excludes every job (queue scope)", () => {
    const onFilterChange = vi.fn()
    const onScopeChange = vi.fn()
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobFilter
          {...baseProps}
          filter={{ name: "nope", error: "" }}
          totalCount={50}
          filteredCount={0}
          onFilterChange={onFilterChange}
          onScopeChange={onScopeChange}
        />
      </Wrapper>,
    )
    expect(screen.getByTestId("job-filter-empty-helper")).toBeInTheDocument()
    fireEvent.click(screen.getByTestId("job-filter-clear"))
    expect(onFilterChange).toHaveBeenCalledWith({ name: "", error: "" })
    fireEvent.click(screen.getByTestId("job-filter-expand"))
    expect(onScopeChange).toHaveBeenCalledWith("cross")
  })

  it("hides the `Expand to all queues` button when scope is already cross", () => {
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobFilter
          {...baseProps}
          scope="cross"
          filter={{ name: "nope", error: "" }}
          filteredCount={0}
          totalCount={50}
        />
      </Wrapper>,
    )
    expect(screen.queryByTestId("job-filter-expand")).not.toBeInTheDocument()
    expect(screen.getByTestId("job-filter-clear")).toBeInTheDocument()
  })

  it("clear-× button on a non-empty name segment resets that segment only", () => {
    const onFilterChange = vi.fn()
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobFilter
          {...baseProps}
          filter={{ name: "send", error: "Connection" }}
          onFilterChange={onFilterChange}
        />
      </Wrapper>,
    )
    fireEvent.click(screen.getByTestId("job-filter-name-clear"))
    expect(onFilterChange).toHaveBeenCalledWith({ name: "", error: "Connection" })
  })

  it("renders the scan note when the server search was capped", () => {
    const Wrapper = wrap()
    const { rerender } = render(
      <Wrapper>
        <JobFilter {...baseProps} />
      </Wrapper>,
    )
    expect(screen.queryByTestId("job-filter-scan-note")).not.toBeInTheDocument()
    rerender(
      <Wrapper>
        <JobFilter {...baseProps} scanNote="Looked at the newest 10,000 failed jobs." />
      </Wrapper>,
    )
    expect(screen.getByTestId("job-filter-scan-note")).toHaveTextContent(/newest 10,000/)
    expect(screen.getByLabelText("Filter by job name or id")).toBeInTheDocument()
  })
})
