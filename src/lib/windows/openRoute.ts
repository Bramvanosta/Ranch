import { toast } from "sonner"
import { unwrap } from "@/data/result"
import { hasNativeWindows } from "@/lib/runtime/runtime"
import { commands } from "@/lib/tauri/bindings"

/**
 * Show an app route in a new window, or in a new tab of this one
 *. The desktop app opens a native window (a native tab on macOS); a
 * browser opens a browser tab, which is what a window is there.
 */
export async function openRoute(route: string, { tab }: { tab: boolean }) {
  if (hasNativeWindows()) {
    unwrap(await commands.openWindow({ route, tab }))
    return
  }
  window.open(route, "_blank", "noopener")
}

/**
 * The window the app started with. Only it remembers the last connection
 * for the next launch: extra windows and tabs come and go.
 */
export function isMainWindow(): boolean {
  if (!hasNativeWindows()) return true
  const label = currentWindowLabel()
  return label === undefined || label === "main"
}

/** The Tauri label of this window (`main`, `settings`, `win-…`); none in a browser. */
export function currentWindowLabel(): string | undefined {
  return (window as { __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } } })
    .__TAURI_INTERNALS__?.metadata?.currentWindow?.label
}

/** `openRoute` from a click or a menu: a failure becomes a toast, not an unhandled rejection. */
export function openRouteOrSay(route: string, how: { tab: boolean }) {
  openRoute(route, how).catch((e: unknown) =>
    toast.error(e instanceof Error ? e.message : "Could not open a window"),
  )
}

/** The route of a queue, of a job. */
export const queueRoute = (connectionId: string, queueId: string) =>
  `/connections/${encodeURIComponent(connectionId)}/queues/${encodeURIComponent(queueId)}`
export const jobRoute = (connectionId: string, queueId: string, jobId: string) =>
  `${queueRoute(connectionId, queueId)}/${encodeURIComponent(jobId)}`
