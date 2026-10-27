import { cn } from "@/lib/utils"

/**
 * The top bar of a screen (54 px): where you are, what it is, and
 * the screen's actions on the right. Empty space drags the window on macOS.
 * A long title wraps (the bar grows) instead of being cut.
 */
export function Toolbar({
  title,
  leading,
  meta,
  actions,
  titleTestId,
  titleAs = "h1",
  className,
}: {
  title: React.ReactNode
  /** Before the title, e.g. a "← Queues" link. */
  leading?: React.ReactNode
  /** Right after the title: environment tag, key, state. */
  meta?: React.ReactNode
  actions?: React.ReactNode
  titleTestId?: string
  /** `div` when the screen's own content carries the page heading. */
  titleAs?: "h1" | "div"
  className?: string
}) {
  const Title = titleAs
  return (
    <header
      data-tauri-drag-region
      data-slot="toolbar"
      className={cn(
        "flex min-h-toolbar shrink-0 flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-(--line) px-5 py-2",
        className,
      )}
    >
      {leading}
      <Title
        className="min-w-0 text-lg leading-tight font-bold break-all text-(--text)"
        data-testid={titleTestId}
      >
        {title}
      </Title>
      {meta ? <div className="flex shrink-0 flex-wrap items-center gap-2">{meta}</div> : null}
      {actions ? (
        <div className="ml-auto flex shrink-0 flex-wrap items-center gap-2">{actions}</div>
      ) : null}
    </header>
  )
}
