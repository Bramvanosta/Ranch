import { cn } from "@/lib/utils"

/**
 * What a screen shows when there is nothing in it: what is
 * empty, why, and the one thing to do about it.
 */
export function EmptyState({
  title,
  description,
  icon,
  action,
  className,
}: {
  title: string
  description?: React.ReactNode
  icon?: React.ReactNode
  action?: React.ReactNode
  className?: string
}) {
  return (
    <section
      aria-label={title}
      data-slot="empty-state"
      className={cn(
        "flex min-w-0 flex-col items-center gap-2 px-6 py-10 text-center text-sm",
        className,
      )}
    >
      {icon ? (
        <span aria-hidden="true" className="text-(--text2) [&_svg]:size-6">
          {icon}
        </span>
      ) : null}
      <p className="max-w-md font-semibold break-words text-(--text)">{title}</p>
      {description ? <p className="max-w-md break-words text-(--text2)">{description}</p> : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </section>
  )
}
