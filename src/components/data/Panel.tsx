import type { ComponentProps } from "react"

import { cn } from "@/lib/utils"

/**
 * A panel: surface, border, radius — only around
 * content that belongs together (a chart, a short ranked list). Lists of
 * things are rows, not panels.
 */
export function Panel({ className, ...props }: ComponentProps<"section">) {
  return (
    <section
      className={cn(
        "flex min-w-0 flex-col rounded-card border border-(--line) bg-(--surf) text-sm text-(--text)",
        className,
      )}
      {...props}
    />
  )
}

/** The panel's title line, with what qualifies it (a range, a count). */
export function PanelHeader({ className, ...props }: ComponentProps<"header">) {
  return <header className={cn("flex flex-col gap-1 px-4 pt-3", className)} {...props} />
}

export function PanelTitle({ className, ...props }: ComponentProps<"h3">) {
  return <h3 className={cn("text-sm font-semibold", className)} {...props} />
}

export function PanelBody({ className, ...props }: ComponentProps<"div">) {
  return <div className={cn("min-w-0 px-4 pt-2 pb-4", className)} {...props} />
}
