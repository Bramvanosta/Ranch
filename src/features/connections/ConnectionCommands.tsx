import { useCommandHandler } from "@/commands/handlers"
import { redisUrl } from "@/domain/bullmqEndpoint"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import type { Connection } from "@/lib/tauri/bindings"

import { useConnectionMenu } from "./useConnectionMenu"

/**
 * The Connection menu for the open connection: Edit (⌘E),
 * Copy Redis URL, Remove. Mounted by the connection route, so it applies on
 * every screen of that connection whatever the sidebar shows.
 */
export function ConnectionCommands({ connection }: { connection: Connection }) {
  const menu = useConnectionMenu(connection)
  const copy = useCopyToClipboard()
  useCommandHandler("connection.edit", menu.openEdit)
  useCommandHandler("connection.remove", menu.openRemove)
  const url = redisUrl(connection)
  useCommandHandler("connection.copyUrl", () => url && copy(url, "Redis URL"), {
    enabled: url !== null,
  })
  return <>{menu.dialogs}</>
}
