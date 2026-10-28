import { cn } from "@/lib/utils"

/**
 * A quiet line of facts under the toolbar (32 px, mono 12):
 * host, port, db, Redis version… Connection views only.
 */
export function InfoStrip({
  children,
  label = "Details",
  className,
}: {
  children: React.ReactNode
  label?: string
  className?: string
}) {
  return (
    <section
      aria-label={label}
      data-slot="info-strip"
      className={cn(
        "flex min-h-info-strip shrink-0 flex-wrap items-center gap-x-4 gap-y-1 border-b border-(--line) bg-(--surf) px-5 py-1 font-mono text-xs text-(--text2)",
        className,
      )}
    >
      {children}
    </section>
  )
}
