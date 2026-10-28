/**
 * MetricCard rendering tests.
 */
import { render, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

// Stub recharts: JSDOM doesn't measure, so ResponsiveContainer
// stays at 0×0 without intervention. We don't care about the
// chart visuals here, just that the sparkline branch renders.
vi.mock("recharts", () => {
  const Passthrough = ({ children }: { children?: React.ReactNode }) => (
    <div data-stub="chart">{children}</div>
  )
  const Leaf = () => <div data-stub="leaf" />
  return {
    ResponsiveContainer: Passthrough,
    LineChart: Passthrough,
    Line: Leaf,
  }
})

import { MetricCard } from "./MetricCard"

describe("MetricCard", () => {
  it("renders the label and value", () => {
    render(<MetricCard label="Throughput" value="42/h" />)
    expect(screen.getByText("Throughput")).toBeInTheDocument()
    expect(screen.getByTestId("metric-card-value")).toHaveTextContent("42/h")
  })

  it("renders an optional hint", () => {
    render(<MetricCard label="Failure rate" value="3%" hint="12 failed · 24h" />)
    expect(screen.getByText(/12 failed/)).toBeInTheDocument()
  })

  it("places the hint BELOW the value", () => {
    render(<MetricCard label="Throughput" value="42/h" hint="below me" />)
    const value = screen.getByTestId("metric-card-value")
    const hint = screen.getByTestId("metric-card-hint")
    // Both should be inside the same CardContent; the hint should be a
    // later sibling than the value (DOM order = visual stacking order).
    expect(value.parentElement).toBe(hint.parentElement)
    expect(value.compareDocumentPosition(hint) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it("applies the value class override", () => {
    render(<MetricCard label="Failure rate" value="15%" valueClassName="text-red-600" />)
    expect(screen.getByTestId("metric-card-value").className).toMatch(/text-red-600/)
  })

  it("renders a sparkline when values are provided", () => {
    const { container } = render(
      <MetricCard label="Throughput" value="42/h" sparkline={[1, 2, 3, 5, 4]} />,
    )
    expect(container.querySelector('[data-stub="chart"]')).not.toBeNull()
  })

  it("does not render a sparkline when no values", () => {
    const { container } = render(<MetricCard label="Throughput" value="42/h" />)
    expect(container.querySelector('[data-stub="chart"]')).toBeNull()
  })

  it("encodes the label into the testid", () => {
    render(<MetricCard label="Avg processing" value="1s" />)
    expect(screen.getByTestId("metric-card-avg-processing")).toBeInTheDocument()
  })
})
