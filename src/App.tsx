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

/**
 * Root application component: the theme (`next-themes`, following the OS
 * until the user picks one), the query client, the tooltip provider, the
 * router and the toaster, plus the bridges every window needs.
 */
export function App() {
  // Single QueryClient per app, created lazily so Vite's HMR doesn't churn it.
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
      // Keyed by queue: a cross-queue bulk run streams one progress per
      // queue, and they must not overwrite each other.
      tick(event.payload.queueId, event.payload.completed, event.payload.failedSoFar)
    })
    return () => {
      promise
        .then((unlisten) => unlisten())
        .catch(() => {
          // Listener teardown errors are non-fatal; the unmount path
          // is best-effort. Tauri cleans up on app exit anyway.
        })
    }
  }, [tick])
  return null
}
