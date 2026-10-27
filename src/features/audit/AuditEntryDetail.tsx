import { useQuery } from "@tanstack/react-query"
import { XIcon } from "lucide-react"

import { CodeBlock } from "@/components/data/CodeBlock"
import { SectionHeader } from "@/components/data/SectionHeader"
import { Button } from "@/components/ui/button"
import { agentIdentity, agentLabel } from "@/features/mcp/agentLabel"
import { formatDurationMs } from "@/features/metrics/format"
import { type AuditLogEntry, commands } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { ACTION_LABELS, formatWhen, whoOf } from "./auditLabels"
import { formatAuditTarget } from "./formatAuditTarget"

/**
 * One audit entry beside the table: when, who, the
 * connection, the result, the approval behind an agent write (who asked,
 * who approved, after how long) and the exact arguments.
 */
export function AuditEntryDetail({
  entry,
  onClose,
}: {
  entry: AuditLogEntry
  onClose: () => void
}) {
  const approval = useQuery({
    queryKey: ["approvalRecord", entry.approvalId],
    queryFn: async () => {
      if (!entry.approvalId) return null
      const r = await commands.getApprovalRecord(entry.approvalId)
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    enabled: Boolean(entry.approvalId),
    retry: false,
  })
  const record = approval.data
  const waited =
    record?.decidedAt && record.createdAt
      ? Date.parse(record.decidedAt) - Date.parse(record.createdAt)
      : null
  const decided =
    record?.status === "approved" || record?.status === "consumed"
      ? "approved by you"
      : record?.status === "denied"
        ? "denied by you"
        : record?.status === "expired"
          ? "expired before anyone decided"
          : "still pending"
  const where = [entry.groupName, entry.environmentName, entry.connectionName]
    .filter(Boolean)
    .join(" / ")

  return (
    <div className="flex min-h-0 flex-col" data-testid="audit-entry-detail">
      <div className="flex items-center justify-end border-b border-(--line) px-2 py-1">
        <Button variant="ghost" size="icon-sm" aria-label="Close the details" onClick={onClose}>
          <XIcon aria-hidden="true" />
        </Button>
      </div>
      <div className="flex flex-col gap-5 p-4">
        <header className="flex flex-col gap-1">
          <h2 className="text-lg font-semibold">{ACTION_LABELS[entry.action]}</h2>
          <p className="font-tabular text-sm break-all">{formatAuditTarget(entry.target)}</p>
        </header>
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-sm">
          <dt className="text-(--text2)">When</dt>
          <dd className="font-tabular" title={entry.createdAt}>
            {formatWhen(entry.createdAt)}
          </dd>
          <dt className="text-(--text2)">Who</dt>
          <dd className="break-words">
            {whoOf(entry)}
            {entry.source === "agent" ? (
              <span className="ml-1 text-xs text-(--text2)">
                {agentIdentity(entry.agentClient, entry.agentTransport)}
              </span>
            ) : null}
          </dd>
          <dt className="text-(--text2)">Connection</dt>
          <dd className="flex min-w-0 items-center gap-2 break-words">
            <span
              aria-hidden="true"
              className="h-4 w-1 shrink-0 rounded-sm"
              style={{
                backgroundColor: entry.envColor ? `var(--${entry.envColor})` : "transparent",
              }}
            />
            {where || "—"}
          </dd>
          <dt className="text-(--text2)">Result</dt>
          <dd
            className={cn(
              "break-words",
              entry.outcome === "failure"
                ? "text-(--state-failed-text)"
                : entry.outcome === "success"
                  ? ""
                  : "text-(--text2)",
            )}
            data-testid="audit-detail-result"
          >
            {entry.outcome === "success"
              ? "Done"
              : entry.outcome === "failure"
                ? "Failed"
                : "Cancelled"}
            {entry.outcomeReason ? ` — ${entry.outcomeReason}` : ""}
          </dd>
        </dl>

        {entry.approvalId ? (
          <section aria-label="Approval" className="flex flex-col gap-2">
            <SectionHeader title="Approval" level={3} />
            {record ? (
              <p className="text-sm" data-testid="audit-detail-approval">
                Asked by {agentLabel(record.clientHint) ?? "an agent"} at{" "}
                {formatWhen(record.createdAt)} · {decided}
                {waited !== null && waited >= 0 ? ` after ${formatDurationMs(waited)}` : ""}
                {record.reason ? ` — “${record.reason}”` : ""}
              </p>
            ) : approval.isLoading ? null : (
              <p className="text-sm text-(--text2)">This approval is no longer kept.</p>
            )}
          </section>
        ) : null}

        <section aria-label="Arguments" className="flex flex-col gap-2">
          <SectionHeader title="Arguments" level={3} />
          <CodeBlock label="Arguments">
            <code data-testid="audit-detail-arguments">
              {pretty(
                record?.argumentsJson ??
                  JSON.stringify(
                    entry.target.kind === "connection"
                      ? { connection: entry.connectionName }
                      : entry.target.data,
                  ),
              )}
            </code>
          </CodeBlock>
        </section>
      </div>
    </div>
  )
}

function pretty(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2)
  } catch {
    return raw
  }
}
