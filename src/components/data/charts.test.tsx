import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { HeatStrip, Sparkline, StackedBar, StatTile } from "./charts"

/**
 * Every graph with empty, flat, spiky and "metrics off" data.
 * jsdom has no layout: "never overflows" is checked structurally — the
 * graph fills its box (`w-full`, `min-w-0`) and draws inside its viewBox.
 */
const EMPTY: number[] = []
const FLAT = [5, 5, 5, 5, 5, 5]
const ZERO = [0, 0, 0, 0]
const SPIKY = [0, 0, 120, 1, 0, 3_400, 0, 2]

function pointsOf(svg: Element): Array<[number, number]> {
  const raw = svg.querySelector("polyline")?.getAttribute("points") ?? ""
  return raw.split(" ").map((p) => p.split(",").map(Number) as [number, number])
}

describe("Sparkline", () => {
  it("draws every value inside its box, oldest on the left", () => {
    render(<Sparkline values={SPIKY} label="Completed" />)
    const svg = screen.getByTestId("sparkline")
    expect(svg).toHaveAttribute("data-state", "data")
    expect(svg.getAttribute("class")).toContain("w-full")
    const pts = pointsOf(svg)
    expect(pts).toHaveLength(SPIKY.length)
    for (const [x, y] of pts) {
      expect(x).toBeGreaterThanOrEqual(0)
      expect(x).toBeLessThanOrEqual(100)
      expect(y).toBeGreaterThanOrEqual(0)
      expect(y).toBeLessThanOrEqual(24)
    }
    expect(svg).toHaveAccessibleName("Completed: from 0 to 2, peak 3400")
  })

  it("a flat series is a straight line; zeros lie on the floor", () => {
    const { rerender } = render(<Sparkline values={FLAT} label="Rate" />)
    expect(new Set(pointsOf(screen.getByTestId("sparkline")).map(([, y]) => y)).size).toBe(1)
    rerender(<Sparkline values={ZERO} label="Rate" />)
    expect(pointsOf(screen.getByTestId("sparkline")).every(([, y]) => y === 22)).toBe(true)
  })

  it("says 'no data' and 'metrics off' instead of drawing zeros", () => {
    const { rerender } = render(<Sparkline values={EMPTY} label="Rate" />)
    expect(screen.getByTestId("sparkline")).toHaveAccessibleName("Rate: no data")
    rerender(<Sparkline values={null} label="Rate" />)
    expect(screen.getByTestId("sparkline")).toHaveAttribute("data-state", "off")
    expect(screen.getByTestId("sparkline")).toHaveAccessibleName("Rate: metrics off")
  })
})

describe("HeatStrip", () => {
  it("one cell per bucket, the failed share in red", () => {
    render(<HeatStrip completed={SPIKY} failed={[0, 0, 0, 1, 0, 600, 0, 0]} label="emails" />)
    expect(screen.getAllByTestId("heat-cell")).toHaveLength(SPIKY.length)
    expect(screen.getAllByTestId("heat-cell-failed")).toHaveLength(2)
    expect(screen.getByTestId("heat-strip")).toHaveAccessibleName(
      "emails: 3523 completed, 601 failed",
    )
    expect(screen.getByTestId("heat-strip").className).toContain("min-w-0")
  })

  it("flat and empty series stay a strip of cells", () => {
    const { rerender } = render(<HeatStrip completed={FLAT} failed={[]} label="q" />)
    expect(screen.getAllByTestId("heat-cell")).toHaveLength(FLAT.length)
    rerender(<HeatStrip completed={EMPTY} failed={EMPTY} label="q" />)
    expect(screen.getByTestId("heat-strip")).toHaveAttribute("data-state", "empty")
  })

  it("metrics off is hatched and says so", () => {
    render(<HeatStrip completed={[]} failed={[]} metricsOff label="reports" />)
    const strip = screen.getByTestId("heat-strip")
    expect(strip).toHaveAttribute("data-state", "off")
    expect(strip).toHaveTextContent("metrics off")
    expect(strip).toHaveAccessibleName("reports: metrics off")
  })
})

describe("StackedBar", () => {
  const seg = (key: string, value: number) => ({ key, label: key, value, color: "red" })

  it("segments in proportion; zero parts are left out", () => {
    render(
      <StackedBar
        segments={[seg("Waiting", 30), seg("Active", 0), seg("Failed", 10)]}
        label="State mix"
      />,
    )
    expect(screen.getByTestId("stacked-bar-Waiting")).toHaveStyle({ width: "75%" })
    expect(screen.getByTestId("stacked-bar-Failed")).toHaveStyle({ width: "25%" })
    expect(screen.queryByTestId("stacked-bar-Active")).not.toBeInTheDocument()
    expect(screen.getByTestId("stacked-bar")).toHaveAccessibleName(
      "State mix: 30 waiting, 10 failed",
    )
  })

  it("an empty whole is an empty track", () => {
    render(<StackedBar segments={[seg("Waiting", 0)]} label="State mix" />)
    expect(screen.getByTestId("stacked-bar")).toHaveAccessibleName("State mix: none")
  })
})

describe("StatTile", () => {
  it("label, mono figure, sub-line and trend; a long figure wraps", () => {
    render(
      <StatTile
        label="Failed"
        value="1,234,567,890"
        sub="+12 in the last hour"
        trend={SPIKY}
        tone="danger"
      />,
    )
    const tile = screen.getByTestId("stat-tile")
    expect(screen.getByTestId("stat-tile-value")).toHaveTextContent("1,234,567,890")
    expect(screen.getByTestId("stat-tile-value").className).toContain("break-all")
    expect(screen.getByTestId("stat-tile-sub")).toHaveTextContent("+12 in the last hour")
    expect(tile.className).toContain("min-w-0")
    expect(screen.getByTestId("sparkline")).toHaveAttribute("data-state", "data")
  })

  it("metrics off shows in the trend, not as a zero line", () => {
    render(<StatTile label="Completed / h" value="—" trend={null} />)
    expect(screen.getByTestId("sparkline")).toHaveAttribute("data-state", "off")
  })
})
