import type { CommandResult } from "@/data/result"
import { commands, type FollowConnectionEventsArgs } from "@/lib/tauri/bindings"

/**
 * Follow a connection's event streams, or stop (`follow_connection_events`).
 * `true` when the connection is followed; a runtime without live events
 * answers `false`.
 */
export function followConnectionEvents(
  args: FollowConnectionEventsArgs,
): Promise<CommandResult<boolean>> {
  return commands.followConnectionEvents(args)
}
