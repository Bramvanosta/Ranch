import { TriangleAlertIcon } from "lucide-react"

import { formatAge } from "@/features/jobs/formatAge"
import { cn } from "@/lib/utils"

/**
 * One rule for errors: when a refresh fails while data
 * is on screen, the data stays and this line says it is not current. The
 * full error box is only for a view that has nothing to show.
 */
export function StaleNotice({
  error,
  updatedAt,
  what,
  className,
}: {
  error: unknown
  /** When the data on screen was read (`dataUpdatedAt`). */
  updatedAt: number
  /** What could not be refreshed, e.g. "the queues". */
  what: string
  className?: string
}) {
  const age = updatedAt > 0 ? formatAge(Date.now() - updatedAt) : null
  const reason = error instanceof Error ? error.message : typeof error === "string" ? error : ""
  return (
    <div
      role="status"
      data-testid="stale-notice"
      className={cn(
        "flex items-start gap-2 rounded-md border border-(--danger)/30 bg-(--danger)/5 px-3 py-2 text-xs text-(--state-failed-text)",
        className,
      )}
    >
      <TriangleAlertIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
      <p>
        Could not refresh {what}
        {age && age !== "now" ? ` — showing what was read ${age} ago` : ""}. Retrying.
        {reason ? <span className="block text-(--text2)">{reason}</span> : null}
      </p>
    </div>
  )
}
