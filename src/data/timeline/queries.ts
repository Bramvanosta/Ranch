import { useQuery } from "@tanstack/react-query"
import { unwrap } from "@/data/result"
import { commands } from "@/lib/tauri/bindings"
import { timelineKeys } from "./keys"

/**
 * The Timeline's selected window (`from`–`to`, ms since epoch): done, failed,
 * wait p95 against the rest of the range from `rangeFromMs`, failures by cause.
 */
export function useWindowSummary(
  connectionId: string,
  window: { from: number; to: number },
  rangeFromMs: number,
) {
  return useQuery({
    queryKey: timelineKeys.window(connectionId, window.from, window.to, rangeFromMs),
    queryFn: async () => {
      return unwrap(
        await commands.summarizeWindow({
          connectionId,
          fromMs: window.from,
          toMs: window.to,
          rangeFromMs,
        }),
      )
    },
  })
}
