import { Dialog as DialogPrimitive } from "@base-ui/react/dialog"
import type { ReactNode, RefObject } from "react"

/**
 * The palette's own dialog (the one centred dialog left):
 * high in the window over a scrim, no close button, the search field
 * focused. Everything else that asks something is a sheet.
 */
export function PaletteDialog({
  open,
  onOpenChange,
  initialFocus,
  children,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  initialFocus: RefObject<HTMLElement | null>
  children: ReactNode
}) {
  return (
    <DialogPrimitive.Root open={open} onOpenChange={(next) => onOpenChange(next)}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Backdrop className="fixed inset-0 z-50 bg-(--scrim)" />
        <DialogPrimitive.Popup
          initialFocus={initialFocus}
          data-testid="command-palette"
          className="fixed top-[14%] left-1/2 z-50 flex max-h-[70vh] w-[min(37.5rem,calc(100vw-2rem))] -translate-x-1/2 flex-col overflow-hidden rounded-sheet border border-(--line) bg-(--panel) text-(--text) shadow-2xl outline-none"
        >
          <DialogPrimitive.Title className="sr-only">Command palette</DialogPrimitive.Title>
          <DialogPrimitive.Description className="sr-only">
            Go to a queue or a connection, run an action, or find a job by its id.
          </DialogPrimitive.Description>
          {children}
        </DialogPrimitive.Popup>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}
