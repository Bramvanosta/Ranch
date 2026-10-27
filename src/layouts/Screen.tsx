import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable"
import { useMediaQuery } from "@/hooks/useMediaQuery"
import { cn } from "@/lib/utils"

/**
 * One screen of the AppShell: toolbar (54), optional banner and
 * info strip (32), optional view tabs (40), then the content and, when the
 * route provides one, the inspector on the right:
 * a column resizable from 320 to 440 px, or — below 1280 px — a panel over
 * the list, so the list keeps its width.
 */
export function Screen({
  toolbar,
  banner,
  infoStrip,
  tabs,
  inspector,
  children,
  contentClassName,
}: {
  toolbar: React.ReactNode
  /** Something that must be seen before the content (the selection bar). */
  banner?: React.ReactNode
  infoStrip?: React.ReactNode
  tabs?: React.ReactNode
  inspector?: React.ReactNode
  children: React.ReactNode
  contentClassName?: string
}) {
  const wide = useMediaQuery("(min-width: 1280px)")
  const content = (
    <div
      className={cn(
        "flex h-full min-h-0 min-w-0 flex-1 flex-col gap-3 px-5 py-4",
        contentClassName,
      )}
    >
      {children}
    </div>
  )
  const aside = (className?: string) => (
    <aside
      aria-label="Inspector"
      data-testid="inspector"
      className={cn("flex h-full flex-col overflow-auto bg-(--bg)", className)}
    >
      {inspector}
    </aside>
  )

  return (
    <div className="flex h-full min-h-0 flex-col" data-slot="screen">
      {toolbar}
      {banner}
      {infoStrip}
      {tabs}
      <div className="relative flex min-h-0 flex-1">
        {inspector && wide ? (
          <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1">
            <ResizablePanel id="screen-content" minSize="400px">
              {content}
            </ResizablePanel>
            <ResizableHandle />
            <ResizablePanel
              id="screen-inspector"
              defaultSize="380px"
              minSize="320px"
              maxSize="440px"
            >
              {aside("border-l border-(--line)")}
            </ResizablePanel>
          </ResizablePanelGroup>
        ) : (
          <>
            {content}
            {inspector
              ? aside(
                  "absolute inset-y-0 right-0 z-20 w-inspector max-w-full border-l border-(--line) shadow-xl",
                )
              : null}
          </>
        )}
      </div>
    </div>
  )
}
