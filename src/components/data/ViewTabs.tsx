import { Tabs as TabsPrimitive } from "@base-ui/react/tabs"

import { cn } from "@/lib/utils"

export interface ViewTab {
  value: string
  label: string
  /** Shown after the label in mono; `tone: "danger"` for failures. */
  count?: number | string
  tone?: "default" | "danger"
  testId?: string
  countTestId?: string
}

/**
 * The views of a screen (40 px, underline). Must sit inside the
 * screen's `ViewTabsRoot`, whose `ViewPanel`s hold the views.
 */
export function ViewTabs({ items, className }: { items: ViewTab[]; className?: string }) {
  return (
    <TabsPrimitive.List
      data-slot="view-tabs"
      className={cn(
        // Many views (a queue: seven states and four more) wrap to a second
        // line rather than run off the window.
        "flex min-h-view-tabs shrink-0 flex-wrap items-stretch gap-x-4 border-b border-(--line) px-5",
        className,
      )}
    >
      {items.map((tab) => (
        <TabsPrimitive.Tab
          key={tab.value}
          value={tab.value}
          data-testid={tab.testId}
          className="relative inline-flex h-view-tabs shrink-0 items-center gap-1.5 text-sm font-medium whitespace-nowrap text-(--text2) outline-none hover:text-(--text) focus-visible:text-(--text) focus-visible:underline data-active:text-(--text) data-active:after:absolute data-active:after:inset-x-0 data-active:after:bottom-0 data-active:after:h-0.5 data-active:after:rounded-full data-active:after:bg-(--accent)"
        >
          {tab.label}
          {tab.count !== undefined ? (
            <span
              data-testid={tab.countTestId}
              className={cn(
                "font-tabular text-xs",
                tab.tone === "danger" ? "text-(--state-failed-text)" : "text-(--text2)",
              )}
            >
              {tab.count}
            </span>
          ) : null}
        </TabsPrimitive.Tab>
      ))}
    </TabsPrimitive.List>
  )
}

/**
 * The root that holds a screen's views: `ViewTabs` and one
 * `ViewPanel` per view (in place of the shadcn `Tabs`).
 */
export function ViewTabsRoot({ className, ...props }: TabsPrimitive.Root.Props) {
  return <TabsPrimitive.Root className={cn("flex flex-col", className)} {...props} />
}

/**
 * One view's content, shown while its tab is selected. `relative`: what it positions (screen-reader labels, chart
 * tooltips) is placed inside it, so it scrolls with it instead of stretching
 * the window below its content (the window scrolled as a whole).
 */
export function ViewPanel({ className, ...props }: TabsPrimitive.Panel.Props) {
  return <TabsPrimitive.Panel className={cn("relative outline-none", className)} {...props} />
}
