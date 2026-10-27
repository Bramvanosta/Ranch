import { useQuery } from "@tanstack/react-query"
import { useEffect, useState } from "react"
import { toast } from "sonner"

import { EmptyState } from "@/components/data/EmptyState"
import { SectionHeader } from "@/components/data/SectionHeader"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { agentLabel } from "@/features/mcp/agentLabel"
import {
  describeRequest,
  isDestructive,
  secondsLeft,
} from "@/features/mcp/approval/describeApproval"
import { ApprovalDetails } from "@/features/mcp/approval/McpApprovalModal"
import { useDecideApproval, usePendingApprovals } from "@/features/mcp/approval/usePendingApprovals"
import { type ApprovalHistoryItem, commands, type PendingApproval } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

export const APPROVAL_HISTORY_QUERY_KEY = ["mcpApprovalHistory"] as const

/**
 * Agents › Approvals: the requests waiting for you, each
 * with what it would do, the jobs it touches, the exact call and the time
 * left, then the earlier decisions.
 */
export function ApprovalsTab() {
  const pending = usePendingApprovals()
  const history = useQuery({
    queryKey: APPROVAL_HISTORY_QUERY_KEY,
    queryFn: async () => {
      const r = await commands.listApprovalHistory(50)
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    refetchInterval: 10_000,
  })
  return (
    <div className="flex flex-col gap-6" data-testid="agents-approvals">
      <section className="flex flex-col gap-3">
        <SectionHeader title="Waiting for you" count={pending.length} />
        {pending.length === 0 ? (
          <p className="text-sm text-(--text2)">
            Nothing is waiting. A write from an agent shows here, and as a sheet on top of Ranch,
            until you decide.
          </p>
        ) : (
          pending.map((approval) => <PendingCard key={approval.id} approval={approval} />)
        )}
      </section>
      <section className="flex flex-col gap-3">
        <SectionHeader title="Earlier decisions" count={history.data?.length} />
        {history.isError ? (
          <p className="text-sm text-(--state-failed-text)">
            Could not read the earlier decisions.
          </p>
        ) : history.data && history.data.length === 0 ? (
          <EmptyState
            title="No decision yet"
            description="Approvals and denials are kept for 90 days."
          />
        ) : (
          <ul className="divide-y divide-(--line) rounded-card border border-(--line)">
            {(history.data ?? []).map((item) => (
              <HistoryRow key={item.id} item={item} />
            ))}
          </ul>
        )}
      </section>
    </div>
  )
}

function PendingCard({ approval }: { approval: PendingApproval }) {
  const decide = useDecideApproval()
  const [reason, setReason] = useState("")
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [])
  const left = secondsLeft(approval, now)
  const run = (decision: "approve" | "deny") =>
    decide.mutate(
      { approvalId: approval.id, decision, reason: decision === "deny" ? reason : undefined },
      {
        onError: (err) =>
          toast.error(
            `Could not record your decision: ${err instanceof Error ? err.message : "unknown error"}`,
          ),
      },
    )
  return (
    <article
      className="flex flex-col gap-4 rounded-card border border-(--ai-line) bg-(--ai-bg) p-4"
      data-testid="agents-pending-approval"
    >
      <header className="flex flex-wrap items-baseline justify-between gap-2">
        <h3 className="min-w-0 font-semibold break-words">{describeRequest(approval)}</h3>
        <span className="font-tabular text-xs text-(--text2)">
          {left > 0 ? `The agent gives up in ${left}s` : "Timing out"}
        </span>
      </header>
      <ApprovalDetails approval={approval} />
      <div className="grid gap-1.5">
        <label htmlFor={`reason-${approval.id}`} className="text-xs text-(--text2)">
          Reason (optional, sent to the agent if you deny)
        </label>
        <Textarea
          id={`reason-${approval.id}`}
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          rows={2}
          disabled={decide.isPending}
        />
      </div>
      <div className="flex justify-end gap-2">
        <Button variant="default" disabled={decide.isPending} onClick={() => run("deny")}>
          Deny
        </Button>
        <Button
          variant={isDestructive(approval.tool) ? "danger" : "primary"}
          disabled={decide.isPending}
          onClick={() => run("approve")}
        >
          Approve once
        </Button>
      </div>
    </article>
  )
}

const OUTCOME: Record<string, { label: string; className: string }> = {
  approved: { label: "Approved", className: "text-(--state-completed-text)" },
  consumed: { label: "Approved, ran", className: "text-(--state-completed-text)" },
  denied: { label: "Denied", className: "text-(--state-failed-text)" },
  expired: { label: "Nobody answered", className: "text-(--text2)" },
}

function HistoryRow({ item }: { item: ApprovalHistoryItem }) {
  const outcome = OUTCOME[item.status] ?? { label: item.status, className: "text-(--text2)" }
  return (
    <li
      className="grid grid-cols-[7rem_minmax(0,1fr)_auto] items-start gap-x-4 px-3 py-2 text-sm"
      data-testid="agents-history-row"
    >
      <span className="font-tabular text-xs text-(--text2)" title={item.createdAt}>
        {new Date(item.createdAt).toLocaleString([], {
          month: "short",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
        })}
      </span>
      <span className="flex min-w-0 flex-col">
        <span className="break-words">
          {agentLabel(item.clientHint) ?? "An agent"} ·{" "}
          <span className="font-mono text-xs">{item.tool}</span>
        </span>
        <span className="text-xs break-words text-(--text2)">
          {item.connectionName ?? "a removed connection"}
          {item.reason ? ` · “${item.reason}”` : ""}
        </span>
      </span>
      <span className={cn("text-xs font-medium", outcome.className)}>{outcome.label}</span>
    </li>
  )
}
