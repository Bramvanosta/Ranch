import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { RouterProvider } from "@tanstack/react-router"
import { listen } from "@tauri-apps/api/event"
import { ThemeProvider } from "next-themes"
import { useEffect, useState } from "react"

import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { router } from "@/lib/router"
import type { BulkProgressEvent } from "@/lib/tauri/bindings"
import { useBulkProgressStore } from "@/stores/useBulkProgressStore"
import { useJobSelectionStore } from "@/stores/useJobSelectionStore"
import { useSelectionStore } from "@/stores/useSelectionStore"

/**
 * Root application component.
 *
 * Stories:
 *   - 1.1: scaffolded the canonical RailLayout shell with placeholder content.
 *   - 1.4: introduced TanStack Query provider + sonner Toaster.
 *   - 1.5: introduced TanStack Router (file-based routing); App now mounts
 *     `<RouterProvider/>` instead of `<RailLayout/>` directly. The router's
 *     root route (src/routes/__root.tsx) wraps RailLayout around the Outlet.
 *     Also adds `<TooltipProvider/>` for the reachability indicator.
 *   - 3.1 polish: `next-themes` ThemeProvider follows the OS color-scheme
 *     preference automatically. `globals.css` already defines the full
 *     dark-mode token palette under `.dark { ... }` — wiring the
 *     provider here lights everything up at once.
 *     Architecture line 203: "Dark mode is the visual reference."
 *     Toggle UI lives in a future Settings story; for now the app
 *     follows the OS without ceremony.
 */
export function App() {
  // Single QueryClient per app — created lazily so Vite's HMR doesn't churn it.
  const [queryClient] = useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: {
            // Connection list rarely changes; stale-time avoids needless refetches.
            staleTime: 30_000,
            refetchOnWindowFocus: false,
            // Once: a polled view's next poll is its next try,
            // and a failure shows after one retry, not after three.
            retry: 1,
          },
        },
      }),
  )

  return (
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem disableTransitionOnChange>
      <QueryClientProvider client={queryClient}>
        <TooltipProvider delay={250}>
          <BulkProgressBridge />
          <SelectionScopeBridge />
          <RouterProvider router={router} />
          <Toaster />
        </TooltipProvider>
      </QueryClientProvider>
    </ThemeProvider>
  )
}

/**
 * Subscribes once to the actor's `bulk:progress` event
 * bus and feeds tick updates into `useBulkProgressStore`. Renders
 * nothing. Mounted at the app root so the subscription survives
 * route transitions during a long bulk run.
 */
function BulkProgressBridge() {
  const tick = useBulkProgressStore((s) => s.tick)
  useEffect(() => {
    const promise = listen<BulkProgressEvent>("bulk:progress", (event) => {
      // The event payload now carries `queueName` so
      // concurrent per-queue progress streams during a cross-queue
      // bulk run don't stomp on each other.
      tick(event.payload.queueName, event.payload.completed, event.payload.failedSoFar)
    })
    return () => {
      promise
        .then((unlisten) => unlisten())
        .catch(() => {
          // Listener teardown errors are non-fatal — the unmount path
          // is best-effort. Tauri cleans up on app exit anyway.
        })
    }
  }, [tick])
  return null
}

/**
 * Clears the job selection whenever the user switches
 * connection. The selection is scoped to "current connection + queue +
 * state-filter"; the route's effect handles queue+state, this bridge
 * handles connection.
 */
function SelectionScopeBridge() {
  const connectionId = useSelectionStore((s) => s.selectedConnectionId)
  const clear = useJobSelectionStore((s) => s.clear)
  // biome-ignore lint/correctness/useExhaustiveDependencies: connectionId is the intentional trigger — fires when the user switches connection
  useEffect(() => {
    clear()
  }, [connectionId, clear])
  return null
}
