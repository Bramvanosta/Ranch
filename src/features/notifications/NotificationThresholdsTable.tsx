/**
 * Settings › Alerts thresholds editor.
 *
 * Inline table for adding / removing notification thresholds. Saves
 * each row immediately via `useThresholdsState`'s persist helper.
 */
import { useQueryClient } from "@tanstack/react-query"
import { PlusIcon, XIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useConnections } from "@/data/connections/queries"
import { queueKeys } from "@/data/queues/keys"
import type { NotificationThreshold, ThresholdMetric } from "@/domain/thresholds"
import type { Queue } from "@/lib/tauri/bindings"
import { useThresholdsState } from "./useThresholdWatcher"

const METRICS: ThresholdMetric[] = ["failed", "active", "waiting"]

const SELECT_CLASS =
  "h-9 w-full rounded-md border border-(--line) bg-(--bg) px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)"

export function NotificationThresholdsTable() {
  const { thresholds, loaded, persist } = useThresholdsState()
  const queryClient = useQueryClient()
  // The connection is picked from a list, by name.
  const { data: connections } = useConnections()

  if (!loaded) {
    return <p className="text-sm text-(--text2)">Loading thresholds…</p>
  }

  const add = () => {
    const id = `t-${Date.now()}-${Math.floor(Math.random() * 1000)}`
    void persist([...thresholds, { id, metric: "failed", op: "gt", value: 10 }])
  }

  const update = (id: string, patch: Partial<NotificationThreshold>) => {
    void persist(thresholds.map((t) => (t.id === id ? { ...t, ...patch } : t)))
  }

  const remove = (id: string) => {
    void persist(thresholds.filter((t) => t.id !== id))
  }

  return (
    <div className="flex flex-col gap-3" data-testid="notification-thresholds-table">
      <p className="text-sm text-(--text2)">
        Get a native OS notification when a queue counter goes above a threshold. Off until you add
        one; leave the connection and the queue on “Any” to watch everything.
      </p>
      {thresholds.length === 0 ? (
        <p className="rounded-md border border-dashed bg-(--surf)/20 p-4 text-sm text-(--text2)">
          No thresholds configured. Click <em>Add threshold</em> below.
        </p>
      ) : (
        <table className="w-full text-sm" data-testid="thresholds-table">
          <thead className="text-left text-xs text-(--text2)">
            <tr>
              <th className="px-2 py-1 font-medium">Connection</th>
              <th className="px-2 py-1 font-medium">Queue</th>
              <th className="px-2 py-1 font-medium">Counter</th>
              <th className="px-2 py-1 font-medium">Notify above</th>
              <th className="w-10 px-2 py-1" />
            </tr>
          </thead>
          <tbody>
            {thresholds.map((t) => (
              <tr key={t.id} data-testid={`threshold-row-${t.id}`}>
                <td className="px-2 py-1">
                  <select
                    value={t.connectionId ?? ""}
                    onChange={(e) => update(t.id, { connectionId: e.target.value || undefined })}
                    aria-label="Connection"
                    className={SELECT_CLASS}
                    data-testid={`threshold-connection-${t.id}`}
                  >
                    <option value="">Any connection</option>
                    {(connections ?? []).map((c) => (
                      <option key={c.id} value={c.id}>
                        {c.name} ({c.groupName} / {c.envLabel})
                      </option>
                    ))}
                    {t.connectionId && !(connections ?? []).some((c) => c.id === t.connectionId) ? (
                      <option value={t.connectionId}>(removed connection)</option>
                    ) : null}
                  </select>
                </td>
                <td className="px-2 py-1">
                  <Input
                    placeholder="Any queue"
                    value={t.queueName ?? ""}
                    onChange={(e) => update(t.id, { queueName: e.target.value || undefined })}
                    aria-label="Queue"
                    list={`threshold-queues-${t.id}`}
                  />
                  {/* Suggestions: the queues Ranch already listed for that connection. */}
                  <datalist id={`threshold-queues-${t.id}`}>
                    {(t.connectionId
                      ? (queryClient.getQueryData<Queue[]>(queueKeys.list(t.connectionId)) ?? [])
                      : []
                    ).map((q) => (
                      <option key={q.name} value={q.name} />
                    ))}
                  </datalist>
                </td>
                <td className="px-2 py-1">
                  <select
                    value={t.metric}
                    onChange={(e) => update(t.id, { metric: e.target.value as ThresholdMetric })}
                    aria-label="Counter"
                    className={SELECT_CLASS}
                    data-testid={`threshold-metric-${t.id}`}
                  >
                    {METRICS.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                </td>
                <td className="px-2 py-1">
                  <Input
                    type="number"
                    min={0}
                    value={t.value}
                    onChange={(e) => update(t.id, { value: Number(e.target.value) })}
                    data-testid={`threshold-value-${t.id}`}
                  />
                </td>
                <td className="px-2 py-1">
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => remove(t.id)}
                    aria-label="Remove threshold"
                    data-testid={`threshold-remove-${t.id}`}
                  >
                    <XIcon className="h-3.5 w-3.5" />
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div>
        <Button type="button" size="sm" variant="default" onClick={add} data-testid="threshold-add">
          <PlusIcon className="mr-1.5 h-3.5 w-3.5" />
          Add threshold
        </Button>
      </div>
    </div>
  )
}
