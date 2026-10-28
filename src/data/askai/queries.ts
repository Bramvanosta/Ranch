import { unwrap } from "@/data/result"
import { commands } from "@/lib/tauri/bindings"

/**
 * `value` as an agent may see it: job data goes through the
 * `share_for_agents` policy, as it would over MCP.
 */
export async function shareForAgents(
  value: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const r = unwrap(await commands.shareForAgents(JSON.stringify(value)))
  return JSON.parse(r) as Record<string, unknown>
}
