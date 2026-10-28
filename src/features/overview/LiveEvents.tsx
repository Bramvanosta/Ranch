import { Link } from "@tanstack/react-router"
import { PauseIcon, PlayIcon } from "lucide-react"
import { useEffect, useMemo, useState } from "react"

import { displayChord } from "@/commands/chords"
import { runCommand } from "@/commands/handlers"
import { Kbd } from "@/components/data/Kbd"
import { SectionHeader } from "@/components/data/SectionHeader"
import { Button } from "@/components/ui/button"
import { useLiveEvents } from "@/data/events/queries"
import { useDecideApproval } from "@/data/mcp/mutations"
import { useConnectionAgentActivity, usePendingApprovals } from "@/data/mcp/queries"
import { ActivityStatusChip, agentLabel, describeRequest, secondsLeft } from "@/features/mcp"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import type { ActivityEvent, PendingApproval, QueueEvent } from "@/lib/tauri/bindings"
import { formatClock } from "@/lib/time"
import { cn } from "@/lib/utils"

const DOT: Record<string, string> = {
  added: "bg-(--state-waiting)",
  retried: "bg-(--state-waiting)",
  active: "bg-(--state-active)",
  completed: "bg-(--state-completed)",
  failed: "bg-(--state-failed)",
  delayed: "bg-(--state-delayed)",
  stalled: "bg-(--state-delayed)",
  removed: "bg-(--text2)",
}

type Line =
  | { kind: "job"; at: number; key: string; event: QueueEvent }
  | { kind: "agent"; at: number; key: string; activity: ActivityEvent }

const NO_LINES: ActivityEvent[] = []

/**
 * What happens on the connection, as it
 * happens: one line per job event (time, state, job, `queue/id`), agent
 * calls marked ✦ when agents exist, and a pending agent approval on top.
 * Pause freezes the list.
 */
export function LiveEvents({ connectionId }: { connectionId: string }) {
  const aiOn = useAiEnabled()
  const [paused, setPaused] = useState(false)
  const { events, sampled, error } = useLiveEvents(connectionId, paused)
  const agentLines = useAgentLines(connectionId, aiOn && !paused)
  // Lines already fetched leave with AI features too.
  const agents = aiOn ? agentLines : NO_LINES
  const approvals = usePendingApprovalsOf(connectionId)

  const lines = useMemo<Line[]>(() => {
    const jobs: Line[] = events.map((e) => ({
      kind: "job",
      at: e.atMs,
      key: `job:${e.queueName}:${e.streamId}`,
      event: e,
    }))
    const calls: Line[] = agents.map((a) => ({
      kind: "agent",
      at: Date.parse(a.timestamp),
      key: `agent:${a.id}`,
      activity: a,
    }))
    return [...jobs, ...calls].sort((x, y) => y.at - x.at).slice(0, 150)
  }, [events, agents])

  return (
    <section
      aria-label="Live events"
      className="flex min-w-0 flex-col gap-2"
      data-testid="live-events"
    >
      <SectionHeader
        title="Live events"
        actions={
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() => setPaused((p) => !p)}
            aria-pressed={paused}
            data-testid="live-events-pause"
            className="gap-1.5"
          >
            {paused ? <PlayIcon aria-hidden="true" /> : <PauseIcon aria-hidden="true" />}
            {paused ? "Resume" : "Pause"}
          </Button>
        }
      />
      {approvals.map((a) => (
        <ApprovalCard key={a.id} approval={a} />
      ))}
      <div className="rounded-card border border-(--line)">
        {error ? (
          <p role="alert" className="px-3 py-2 text-xs text-(--state-failed-text)">
            {error}
          </p>
        ) : null}
        {sampled ? (
          <p
            className="border-b border-(--line) px-3 py-1.5 text-xs text-(--text2)"
            data-testid="live-events-sampled"
          >
            Busy: some events were skipped, the newest are shown.
          </p>
        ) : null}
        {lines.length === 0 ? (
          <p className="px-3 py-6 text-center text-sm text-(--text2)">
            {paused ? "Paused." : "Waiting for something to happen."}
          </p>
        ) : (
          // The newest lines only: a list scrolling inside the scrolling
          // overview was a second scroll bar for the same page.
          <ol className="divide-y divide-(--line)" data-testid="live-events-list">
            {lines
              .slice(0, LINES_SHOWN)
              .map((line) =>
                line.kind === "job" ? (
                  <JobLine key={line.key} connectionId={connectionId} event={line.event} />
                ) : (
                  <AgentLine key={line.key} activity={line.activity} />
                ),
              )}
          </ol>
        )}
      </div>
    </section>
  )
}

/** Lines shown: about the height of Needs attention beside it. */
const LINES_SHOWN = 12

function JobLine({ connectionId, event }: { connectionId: string; event: QueueEvent }) {
  const name = event.jobName ?? "(job removed)"
  return (
    <li
      className="flex items-start gap-3 px-3 py-1.5 text-sm"
      data-testid="live-event"
      data-event={event.event}
    >
      <span className="shrink-0 font-tabular text-xs whitespace-nowrap text-(--text2)">
        {formatClock(event.atMs)}
      </span>
      <span
        className={cn("mt-1.5 size-2 shrink-0 rounded-full", DOT[event.event] ?? "bg-(--text2)")}
        title={event.event}
        aria-hidden="true"
      />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="flex flex-wrap items-baseline gap-x-2">
          <span className="min-w-0 break-words">
            <span className="sr-only">{event.event}: </span>
            {name}
          </span>
          {event.jobId ? (
            <Link
              to="/connections/$connectionId/queues/$queueId"
              params={{ connectionId, queueId: event.queueId }}
              search={{ job: event.jobId }}
              className="min-w-0 font-tabular text-xs wrap-anywhere text-(--text2) hover:underline"
            >
              {event.queueName}/{event.jobId}
            </Link>
          ) : (
            <span className="font-tabular text-xs text-(--text2)">{event.queueName}</span>
          )}
        </span>
        {event.event === "failed" && event.failedReason ? (
          <span className="font-tabular text-xs break-words text-(--state-failed-text)">
            {event.failedReason.split("\n")[0]}
          </span>
        ) : null}
      </span>
      <span className="shrink-0 text-xs text-(--text2)">{event.event}</span>
    </li>
  )
}

function AgentLine({ activity }: { activity: ActivityEvent }) {
  const who = agentLabel(activity.clientHint) ?? "An agent"
  return (
    <li className="flex items-start gap-3 px-3 py-1.5 text-sm" data-testid="live-event-agent">
      <span className="shrink-0 font-tabular text-xs whitespace-nowrap text-(--text2)">
        {formatClock(activity.timestamp)}
      </span>
      <span className="shrink-0 text-(--ai)">
        <span aria-hidden="true">✦</span>
        <span className="sr-only">Agent</span>
      </span>
      <span className="min-w-0 flex-1 break-words">
        {who} · <span className="font-tabular text-xs">{activity.tool}</span>
      </span>
      {/* The same words, icon and colour as Agents › Activity. */}
      <span className="shrink-0">
        <ActivityStatusChip status={activity.status} />
      </span>
    </li>
  )
}

/** The latest agent calls on this connection (when agents exist). */
function useAgentLines(connectionId: string, enabled: boolean): ActivityEvent[] {
  const { data } = useConnectionAgentActivity(connectionId, enabled)
  return data ?? NO_LINES
}

function usePendingApprovalsOf(connectionId: string): PendingApproval[] {
  const aiOn = useAiEnabled()
  const all = usePendingApprovals()
  return aiOn ? all.filter((a) => a.connectionId === connectionId) : []
}

/** A pending agent request, on top of the list: who, what, how long left. */
function ApprovalCard({ approval }: { approval: PendingApproval }) {
  const decide = useDecideApproval()
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1_000)
    return () => window.clearInterval(id)
  }, [])
  const left = secondsLeft(approval, now)
  return (
    <div
      className="flex flex-wrap items-center gap-3 rounded-card border border-(--ai)/50 bg-(--ai)/10 px-3 py-2 text-sm"
      role="alert"
      data-testid="live-events-approval"
    >
      <span className="text-(--ai)" aria-hidden="true">
        ✦
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="break-words">{describeRequest(approval)}</span>
        <span className="font-tabular text-xs text-(--text2)">
          {approval.tool} · {left} s left
        </span>
      </span>
      <Button
        type="button"
        size="sm"
        variant="default"
        onClick={() => decide.mutate({ approvalId: approval.id, decision: "deny" })}
        disabled={decide.isPending}
        data-testid="live-events-approval-deny"
      >
        Deny
      </Button>
      <Button
        type="button"
        size="sm"
        variant="ai"
        onClick={() => runCommand("agents.pending")}
        data-testid="live-events-approval-review"
        className="gap-1.5"
      >
        Review
        <Kbd>{displayChord("alt+cmd+a")}</Kbd>
      </Button>
    </div>
  )
}
