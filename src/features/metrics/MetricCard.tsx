/**
 * Single dashboard metric card.
 *
 * Layout:
 *   - Subtitle (hint) lives BELOW the value, not beside it.
 *   - Sparkline always aligned to the BOTTOM of the card (flex column
 *     with `mt-auto`) so cards stay visually consistent across the row
 *     regardless of whether the hint wraps.
 *   - Sparkline uses recharts' `LineChart` (same renderer as the
 *     big charts) for a smoother, thinner line — replaces the
 *     hand-rolled SVG polyline.
 */
import { Line, LineChart, ResponsiveContainer } from "recharts"

import { Panel, PanelBody, PanelHeader, PanelTitle } from "@/components/data/Panel"
import { cn } from "@/lib/utils"

interface MetricCardProps {
  label: string
  value: string
  hint?: string
  /** Tailwind class for the value text color (e.g. green/red/blue tints). */
  valueClassName?: string
  /** Optional 24-point sparkline. Numbers oldest-first, index 23 = now. */
  sparkline?: number[]
  /** Sparkline stroke color (CSS variable or hex). */
  sparklineColor?: string
}

export function MetricCard({
  label,
  value,
  hint,
  valueClassName,
  sparkline,
  sparklineColor = "var(--state-completed, #10b981)",
}: MetricCardProps) {
  return (
    <Panel
      data-testid={`metric-card-${label.toLowerCase().replace(/\s+/g, "-")}`}
      className="flex flex-col"
    >
      <PanelHeader className="pb-1">
        <PanelTitle className="text-xs font-medium uppercase tracking-wide text-(--text2)">
          {label}
        </PanelTitle>
      </PanelHeader>
      <PanelBody className="flex flex-1 flex-col gap-1">
        <span
          className={cn("font-tabular text-2xl font-semibold leading-tight", valueClassName)}
          data-testid="metric-card-value"
        >
          {value}
        </span>
        {hint ? (
          <span className="text-xs leading-snug text-(--text2)" data-testid="metric-card-hint">
            {hint}
          </span>
        ) : null}
        {sparkline && sparkline.length > 0 ? (
          <div className="mt-auto pt-2">
            <Sparkline values={sparkline} color={sparklineColor} />
          </div>
        ) : null}
      </PanelBody>
    </Panel>
  )
}

/**
 * Recharts-backed mini line chart. Same renderer the big chart panels
 * use — gives us the same smoothed curve, anti-aliasing and pixel
 * snapping for free, vs. the previous hand-rolled SVG polyline that
 * read as thick and jagged at 24px height.
 */
function Sparkline({ values, color }: { values: number[]; color: string }) {
  // Recharts needs an object array; coerce the bare number[] into rows.
  const data = values.map((v, i) => ({ i, v }))
  return (
    <div className="h-7 w-full" aria-hidden="true">
      <ResponsiveContainer width="100%" height="100%">
        <LineChart data={data} margin={{ top: 2, right: 0, bottom: 2, left: 0 }}>
          <Line
            type="monotone"
            dataKey="v"
            stroke={color}
            strokeWidth={1.25}
            dot={false}
            isAnimationActive={false}
          />
        </LineChart>
      </ResponsiveContainer>
    </div>
  )
}
