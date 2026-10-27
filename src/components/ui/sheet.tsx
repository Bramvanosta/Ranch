import { Dialog as DialogPrimitive } from "@base-ui/react/dialog"
import type { ReactNode, RefObject } from "react"

import { cn } from "@/lib/utils"

/**
 * A sheet: attached to the top
 * of the window like a macOS sheet, over a scrim, focus kept inside, Esc
 * closes. Replaces the centered dialogs (the palette keeps one).
 */
export function Sheet({
  open,
  onOpenChange,
  title,
  description,
  icon,
  children,
  footer,
  initialFocus,
  testId,
  className,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: ReactNode
  description?: ReactNode
  /** A glyph before the title (destructive sheets: a warning). */
  icon?: ReactNode
  children?: ReactNode
  footer?: ReactNode
  /** The element focused when the sheet opens (destructive sheets: Cancel). */
  initialFocus?: RefObject<HTMLElement | null>
  testId?: string
  className?: string
}) {
  return (
    <DialogPrimitive.Root open={open} onOpenChange={(next) => onOpenChange(next)}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Backdrop className="fixed inset-0 z-50 bg-(--scrim)" />
        <DialogPrimitive.Popup
          initialFocus={initialFocus}
          data-testid={testId}
          className={cn(
            "fixed top-0 left-1/2 z-50 flex max-h-[85vh] w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 flex-col rounded-b-sheet border border-t-0 border-(--line) bg-(--panel) text-(--text) shadow-2xl outline-none",
            className,
          )}
        >
          <div className="flex items-start gap-3 px-5 pt-5">
            {icon ? <span className="mt-0.5 shrink-0 [&_svg]:size-5">{icon}</span> : null}
            <div className="flex min-w-0 flex-col gap-1">
              <DialogPrimitive.Title className="text-base font-bold break-words">
                {title}
              </DialogPrimitive.Title>
              {description ? (
                <DialogPrimitive.Description className="text-sm text-(--text2)">
                  {description}
                </DialogPrimitive.Description>
              ) : null}
            </div>
          </div>
          <div className="flex min-h-0 flex-col gap-3 overflow-y-auto px-5 py-4">{children}</div>
          {footer ? (
            <div className="flex flex-wrap items-center justify-end gap-2 border-t border-(--line) px-5 py-3">
              {footer}
            </div>
          ) : null}
        </DialogPrimitive.Popup>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}
