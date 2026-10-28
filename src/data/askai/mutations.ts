import type { CommandResult } from "@/data/result"
import { type AgentApp, commands, type LaunchClaudeCodeArgs } from "@/lib/tauri/bindings"

/**
 * The Ask AI launchers, as plain calls: the flow that hands a prompt over
 * copies it, launches, and says what happened itself.
 */

/** Claude Code in a terminal, in the group's code folder, with the prompt. */
export function launchClaudeCode(args: LaunchClaudeCodeArgs): Promise<CommandResult<null>> {
  return commands.launchClaudeCode(args)
}

/** Open an agent app; the prompt is already on the clipboard. */
export function openAgentApp(agent: AgentApp): Promise<CommandResult<null>> {
  return commands.openAgentApp(agent)
}
