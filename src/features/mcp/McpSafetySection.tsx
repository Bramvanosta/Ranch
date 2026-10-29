import { EnvDot } from "@/components/data/Env"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { useConnections } from "@/data/connections/queries"
import { useSetConnectionMcpReadonly, useSetMcpSafetySettings } from "@/data/mcp/mutations"
import { useMcpSafetySettings } from "@/data/mcp/queries"
import type { ApprovalMode, Connection } from "@/lib/tauri/bindings"
import { accessOf } from "./agentAccess"
import { effectiveMcpReadonly } from "./effectiveMcpReadonly"

/**
 * Agents › Setup & safety (UX: `McpSafetySection`).
 *
 * Agents always read freely; a connection can be made
 * **read-only for agents**, with a global default for the ones that have no
 * explicit setting. The approval mode follows below.
 */
export function McpSafetySection() {
  const settings = useMcpSafetySettings()
  const setSettings = useSetMcpSafetySettings()
  const setConnection = useSetConnectionMcpReadonly()

  const connections = useConnections()

  if (settings.isPending || connections.isPending) {
    return <p className="text-sm text-(--text2)">Loading safety settings</p>
  }
  if (settings.isError || connections.isError) {
    return (
      <p className="rounded-md border border-(--danger)/40 bg-(--danger)/10 px-3 py-2 text-sm text-(--state-failed-text)">
        Could not load the MCP safety settings.
      </p>
    )
  }

  const readonlyDefault = settings.data.readonlyDefault
  const byGroup = groupByPlacement(connections.data)
  const current = settings.data

  return (
    <div className="flex flex-col gap-4" data-testid="mcp-safety-section">
      {/* Approval mode and timeout. Global: the per-connection dimension is
          "read-only" below. */}
      <fieldset className="rounded-md border border-(--line) p-3">
        <legend className="px-1 text-sm font-medium">Ask me before an agent</legend>
        <div role="radiogroup" aria-label="Approval mode" className="flex flex-col gap-2">
          {APPROVAL_MODES.map((mode) => (
            <label key={mode.value} className="flex cursor-pointer items-start gap-2 text-sm">
              <input
                type="radio"
                name="mcp-approval-mode"
                className="mt-1"
                checked={current.approvalMode === mode.value}
                onChange={() => setSettings.mutate({ ...current, approvalMode: mode.value })}
              />
              <span>
                <span className="font-medium">{mode.label}</span>
                <span className="block text-xs text-(--text2)">{mode.hint}</span>
              </span>
            </label>
          ))}
        </div>
        <div className="mt-3 flex items-center gap-2 text-sm">
          <label htmlFor="mcp-approval-timeout">Give up after</label>
          <input
            id="mcp-approval-timeout"
            type="number"
            min={10}
            max={3600}
            defaultValue={current.approvalTimeoutSeconds}
            key={current.approvalTimeoutSeconds}
            disabled={current.approvalMode === "off"}
            onBlur={(e) => {
              const seconds = Number.parseInt(e.target.value, 10)
              if (Number.isFinite(seconds) && seconds !== current.approvalTimeoutSeconds) {
                setSettings.mutate({ ...current, approvalTimeoutSeconds: seconds })
              }
            }}
            className="h-8 w-20 rounded-md border border-(--line) bg-transparent px-2 text-sm disabled:opacity-50"
          />
          <span className="text-(--text2)">
            seconds without an answer (10–3600). The agent is told it timed out.
          </span>
        </div>

        {/* How Ranch gets your attention. */}
        <div className="mt-3 flex flex-col gap-2 border-t border-(--line) pt-3">
          {ATTENTION_SWITCHES.map((item) => (
            <div key={item.key} className="flex items-start justify-between gap-4">
              <div>
                <label htmlFor={`mcp-${item.key}`} className="text-sm font-medium">
                  {item.label}
                </label>
                <p className="text-xs text-(--text2)">{item.hint}</p>
              </div>
              <Switch
                id={`mcp-${item.key}`}
                checked={current[item.key]}
                disabled={current.approvalMode === "off"}
                onCheckedChange={(checked) =>
                  setSettings.mutate({ ...current, [item.key]: checked })
                }
              />
            </div>
          ))}
        </div>
      </fieldset>

      <div className="flex items-start justify-between gap-4 rounded-md border border-(--line) p-3">
        <div>
          <label htmlFor="mcp-readonly-default" className="text-sm font-medium">
            Read-only by default
          </label>
          <p className="text-xs text-(--text2)">
            Connections without their own setting are read-only for agents. Reading is always
            allowed; this only concerns writes such as retry, remove and promote.
          </p>
        </div>
        <Switch
          id="mcp-readonly-default"
          checked={readonlyDefault}
          onCheckedChange={(checked) =>
            setSettings.mutate({ ...current, readonlyDefault: checked })
          }
        />
      </div>

      {connections.data.length === 0 ? (
        <p className="text-sm text-(--text2)">
          No connections yet. Per-connection settings will appear here.
        </p>
      ) : (
        <div className="flex flex-col gap-3">
          {byGroup.map((group) => (
            <section key={group.id} aria-label={group.name}>
              <h4 className="pb-1 text-xs font-medium uppercase tracking-wide text-(--text2)">
                {group.name}
              </h4>
              <ul className="divide-y divide-(--line) rounded-md border border-(--line)">
                {group.connections.map((c) => {
                  const followsDefault = c.mcpReadonly === null
                  return (
                    <li
                      key={c.id}
                      className="flex items-center gap-3 px-3 py-2"
                      data-testid={`mcp-readonly-row-${c.id}`}
                    >
                      <EnvDot color={c.envColor} decorative />
                      <div className="min-w-0 flex-1">
                        <p className="text-sm break-words">{c.name}</p>
                        <p className="text-xs break-words text-(--text2)">{c.envLabel}</p>
                      </div>
                      {/* What agents may do here, in one word. */}
                      <span
                        className="w-16 text-right text-xs font-medium"
                        data-testid={`mcp-access-${c.id}`}
                      >
                        {accessOf(effectiveMcpReadonly(c, readonlyDefault), current.approvalMode)}
                      </span>
                      {followsDefault ? (
                        <span className="text-xs text-(--text2)">(default)</span>
                      ) : (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-6 px-2 text-xs"
                          onClick={() =>
                            setConnection.mutate({ connectionId: c.id, mcpReadonly: null })
                          }
                        >
                          Use default
                        </Button>
                      )}
                      <Switch
                        aria-label={`${c.name}: read-only for agents`}
                        checked={effectiveMcpReadonly(c, readonlyDefault)}
                        onCheckedChange={(checked) =>
                          setConnection.mutate({ connectionId: c.id, mcpReadonly: checked })
                        }
                      />
                    </li>
                  )
                })}
              </ul>
            </section>
          ))}
        </div>
      )}
    </div>
  )
}

const APPROVAL_MODES: { value: ApprovalMode; label: string; hint: string }[] = [
  {
    value: "writes_only",
    label: "Writes anything",
    hint: "Retry, remove, promote and every other write wait for your approval in Ranch. Reading is free. Recommended.",
  },
  {
    value: "all",
    label: "Does anything on a connection",
    hint: "Reads that target a connection, and notes left on a failure cause, wait too. Listing connections and environments stays free.",
  },
  {
    value: "off",
    label: "Never ask",
    hint: "Agents write without asking. Only read-only connections and locked connections stop them.",
  },
]

const ATTENTION_SWITCHES: {
  key: "approvalAlwaysOnTop" | "approvalNotifySound"
  label: string
  hint: string
}[] = [
  {
    key: "approvalAlwaysOnTop",
    label: "Bring Ranch to the front",
    hint: "Un-minimize, focus and keep the window on top until you have decided.",
  },
  {
    key: "approvalNotifySound",
    label: "Notify with a sound",
    hint: "A system notification when Ranch is in the background, and a short sound.",
  },
]

interface PlacementGroup {
  id: string
  name: string
  connections: Connection[]
}

/** `list_connections` already comes in sidebar order; keep it. */
function groupByPlacement(connections: Connection[]): PlacementGroup[] {
  const groups = new Map<string, PlacementGroup>()
  for (const c of connections) {
    let group = groups.get(c.groupId)
    if (!group) {
      group = { id: c.groupId, name: c.groupName, connections: [] }
      groups.set(c.groupId, group)
    }
    group.connections.push(c)
  }
  return Array.from(groups.values())
}
