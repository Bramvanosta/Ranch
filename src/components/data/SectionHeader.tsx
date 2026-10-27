import { cn } from "@/lib/utils"

/**
 * A section title with an optional mono count and actions on the right
 *. The title wraps rather than being cut; actions never shrink.
 */
export function SectionHeader({
  title,
  count,
  actions,
  level = 2,
  className,
}: {
  title: string
  count?: number
  actions?: React.ReactNode
  level?: 2 | 3
  className?: string
}) {
  const Heading = level === 2 ? "h2" : "h3"
  return (
    <header
      data-slot="section-header"
      className={cn("flex min-w-0 items-center justify-between gap-3", className)}
    >
      <Heading className="flex min-w-0 items-baseline gap-2 text-sm font-bold break-words text-(--text)">
        {title}
        {count !== undefined ? (
          <span className="font-tabular text-xs font-normal text-(--text2)">{count}</span>
        ) : null}
      </Heading>
      {actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
    </header>
  )
}
