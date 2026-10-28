import { cn } from "@/lib/utils"

export interface SegmentedOption<T extends string> {
  value: T
  label: string
  testId?: string
}

/**
 * A pill group for filters and ranges: one choice among a few,
 * all visible. Arrow keys move the choice, like native radio groups.
 */
export function Segmented<T extends string>({
  value,
  onChange,
  options,
  label,
  className,
}: {
  value: T
  onChange: (value: T) => void
  options: SegmentedOption<T>[]
  /** The accessible name of the group. */
  label: string
  className?: string
}) {
  const move = (delta: number) => {
    const i = options.findIndex((o) => o.value === value)
    const next = options[(i + delta + options.length) % options.length]
    if (next) onChange(next.value)
  }
  return (
    <div
      role="radiogroup"
      aria-label={label}
      data-slot="segmented"
      className={cn(
        "inline-flex shrink-0 items-center gap-0.5 rounded-control border border-(--line) bg-(--surf) p-0.5",
        className,
      )}
      onKeyDown={(e) => {
        if (e.key === "ArrowRight" || e.key === "ArrowDown") {
          e.preventDefault()
          move(1)
        } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
          e.preventDefault()
          move(-1)
        }
      }}
    >
      {options.map((o) => {
        const selected = o.value === value
        return (
          // biome-ignore lint/a11y/useSemanticElements: a styled segmented control, not native radios
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={selected}
            tabIndex={selected ? 0 : -1}
            data-testid={o.testId}
            onClick={() => onChange(o.value)}
            className={cn(
              "h-6 rounded-[5px] px-2.5 text-xs font-medium whitespace-nowrap outline-none focus-visible:ring-2 focus-visible:ring-(--accent)/50",
              selected
                ? "bg-(--surf2) text-(--text) shadow-xs"
                : "text-(--text2) hover:text-(--text)",
            )}
          >
            {o.label}
          </button>
        )
      })}
    </div>
  )
}
