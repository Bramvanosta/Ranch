import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { ProgressBar } from "./progress-bar"

describe("ProgressBar", () => {
  it("exposes the value and draws the fill to it", () => {
    render(<ProgressBar value={30} max={120} ariaLabel="Progress" />)
    const bar = screen.getByRole("progressbar", { name: "Progress" })
    expect(bar).toHaveAttribute("aria-valuenow", "30")
    expect(bar).toHaveAttribute("aria-valuemax", "120")
    expect((bar.firstElementChild as HTMLElement).style.width).toBe("25%")
  })

  it("keeps the label outside the bar", () => {
    render(<ProgressBar value={42} label="42 %" ariaLabel="Job progress" />)
    const bar = screen.getByRole("progressbar")
    expect(bar).not.toHaveTextContent("42 %")
    expect(screen.getByText("42 %").parentElement).toBe(bar.parentElement)
  })

  it("clamps out-of-range values", () => {
    render(<ProgressBar value={140} ariaLabel="Over" />)
    const bar = screen.getByRole("progressbar")
    expect(bar).toHaveAttribute("aria-valuenow", "100")
    expect((bar.firstElementChild as HTMLElement).style.width).toBe("100%")
  })

  it("is a 6 px track", () => {
    render(<ProgressBar value={1} ariaLabel="Thin" />)
    expect(screen.getByRole("progressbar").className).toContain("h-1.5")
  })
})
