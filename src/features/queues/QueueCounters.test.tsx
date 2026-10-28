import { fireEvent, render, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import type { StateCounts as Counters } from "@/lib/tauri/bindings"

import { QueueCounters } from "./QueueCounters"

const fixture: Counters = {
  waiting: 50,
  active: 3,
  completed: 1234,
  failed: 7,
  delayed: 12,
  prioritized: 0,
  paused: 0,
}

describe("QueueCounters", () => {
  it("renders 5 interactive pills when onSelectState is provided", () => {
    render(<QueueCounters counts={fixture} onSelectState={vi.fn()} />)
    expect(screen.getByRole("button", { name: "Waiting: 50" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Active: 3" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Completed: 1234" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Failed: 7" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Delayed: 12" })).toBeInTheDocument()
  })

  it("renders inert pills (no buttons) when onSelectState is omitted", () => {
    // When QueueCounters is nested inside a Link wrapper
    // (QueueList row), the pills MUST render as `<span>` to avoid the
    // invalid `<button>` inside `<a>` HTML nesting.
    render(<QueueCounters counts={fixture} />)
    expect(screen.queryAllByRole("button")).toHaveLength(0)
    expect(screen.getByTitle("Failed: 7")).toBeInTheDocument()
  })

  // The refresh pulse dot was dropped (visual noise per
  // user feedback). The component should NEVER render the pulse,
  // regardless of `isFetching`. Kept as a regression-style assertion
  // so a future contributor can't accidentally re-add it.
  it("never renders the pulse dot, even with isFetching=true", () => {
    render(<QueueCounters counts={fixture} isFetching />)
    expect(screen.queryByTestId("queue-counters-pulse")).not.toBeInTheDocument()
  })

  it("calls onSelectState with the right state key when a pill is clicked", () => {
    const onSelectState = vi.fn()
    render(<QueueCounters counts={fixture} onSelectState={onSelectState} />)
    fireEvent.click(screen.getByRole("button", { name: "Failed: 7" }))
    expect(onSelectState).toHaveBeenCalledWith("failed")
  })

  it("uses font-tabular on counts to prevent number jitter on poll refresh", () => {
    render(<QueueCounters counts={fixture} onSelectState={vi.fn()} />)
    const failedBtn = screen.getByRole("button", { name: "Failed: 7" })
    // The count number is wrapped in a span carrying `font-tabular`.
    const countSpan = failedBtn.querySelector("span.font-tabular")
    expect(countSpan).not.toBeNull()
    expect(countSpan?.textContent).toBe("7")
  })
})
