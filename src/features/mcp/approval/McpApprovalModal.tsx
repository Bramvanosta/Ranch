import { useEffect, useRef, useState } from "react"

import { CodeBlock } from "@/components/data/CodeBlock"
import { EnvDot } from "@/components/data/Env"
import { Button } from "@/components/ui/button"
import { Sheet } from "@/components/ui/sheet"
import { Textarea } from "@/components/ui/textarea"
import { JsonViewer } from "@/features/jobs/JsonViewer"
import { agentIdentity } from "@/features/mcp/agentLabel"
import { useConnectionLockState } from "@/features/safety/useConnectionLockState"
import type { PendingApproval } from "@/lib/tauri/bindings"

import {
  describeMaintenanceScope,
  describeRequest,
  describeSchedulerTiming,
  isDestructive,
  parsePreview,
  secondsLeft,
  type TargetPreview,
} from "./describeApproval"

interface McpApprovalModalProps {
  /** The request to decide — the oldest pending one. */
  approval: PendingApproval
  /** 1-based position and total, for "1 of 3". */
  position: number
  total: number
  /** The request vanished from the queue (timed out / agent gone) while shown. */
  expired: boolean
  deciding: boolean
  onDecide: (decision: "approve" | "deny", reason?: string) => void
  onAcknowledgeExpired: () => void
}

/**
 * The human's decision on a held agent call (UX: `McpApprovalModal`).
 *
 * Rules: Deny has the default focus, Approve never does. Closing the dialog
 * in any way **denies** (reason "dismissed") — silence must never approve.
 * Everything shown comes from the agent or from Redis and is rendered as
 * plain data.
 */
export function McpApprovalModal({
  approval,
  position,
  total,
  expired,
  deciding,
  onDecide,
  onAcknowledgeExpired,
}: McpApprovalModalProps) {
  const [reason, setReason] = useState("")
  const [now, setNow] = useState(() => Date.now())

  // New request → fresh reason field.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset keyed on the request id
  useEffect(() => setReason(""), [approval.id])

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [])

  const denyRef = useRef<HTMLButtonElement>(null)
  const okRef = useRef<HTMLButtonElement>(null)
  const destructive = isDestructive(approval.tool)
  const left = secondsLeft(approval, now)

  return (
    <Sheet
      open
      onOpenChange={(open) => {
        if (open) return
        if (expired) onAcknowledgeExpired()
        else if (!deciding) onDecide("deny", "dismissed")
      }}
      testId="mcp-approval-modal"
      // Deny is the safe default; OK once nothing is waiting any more.
      initialFocus={expired ? okRef : denyRef}
      title={describeRequest(approval)}
      description={
        expired
          ? "This request is no longer waiting: it timed out or the agent gave up. Nothing was executed."
          : "The agent is waiting for your decision. Nothing happens until you approve."
      }
      footer={
        expired ? (
          <Button ref={okRef} variant="primary" onClick={onAcknowledgeExpired}>
            OK
          </Button>
        ) : (
          <>
            <p className="mr-auto text-xs text-(--text2)" data-testid="mcp-approval-countdown">
              {total > 1 ? `${position} of ${total} · ` : ""}
              {left > 0 ? `The agent gives up in ${left}s` : "Timing out"}
            </p>
            <Button
              ref={denyRef}
              variant="default"
              disabled={deciding}
              onClick={() => onDecide("deny", reason)}
            >
              Deny
            </Button>
            <Button
              variant={destructive ? "danger" : "primary"}
              disabled={deciding}
              onClick={() => onDecide("approve")}
            >
              Approve once
            </Button>
          </>
        )
      }
    >
      {/* A long line in the agent's payload must not widen the sheet: the
          children may shrink (`min-w-0`), the payload viewer scrolls. */}
      <div className="flex min-w-0 flex-col gap-4 [&>*]:min-w-0" aria-live="assertive">
        <ApprovalDetails approval={approval} />

        {expired ? null : (
          <div className="grid gap-1.5">
            <label htmlFor="mcp-approval-reason" className="text-xs text-(--text2)">
              Reason (optional, sent to the agent if you deny)
            </label>
            <Textarea
              id="mcp-approval-reason"
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              rows={2}
              disabled={deciding}
            />
          </div>
        )}
      </div>
    </Sheet>
  )
}

/**
 * What a held agent call would do, as both the approval sheet
 * and Agents › Approvals show it: where, which tool, which agent, the jobs it
 * touches, the exact call, and the note when the connection is protected.
 */
export function ApprovalDetails({ approval }: { approval: PendingApproval }) {
  const preview = parsePreview(approval.targetPreviewJson)
  // No transport on a pending request — do not claim one.
  const identity = agentIdentity(approval.clientHint)
  const { data: lock } = useConnectionLockState(approval.connectionId)
  let call = approval.argumentsJson
  try {
    call = JSON.stringify(JSON.parse(approval.argumentsJson), null, 2)
  } catch {
    // shown as sent
  }
  return (
    <div className="flex min-w-0 flex-col gap-4 [&>*]:min-w-0">
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-sm">
        <dt className="text-(--text2)">Connection</dt>
        <dd className="flex min-w-0 items-start gap-2">
          <EnvDot color={approval.envColor} decorative className="mt-1.5" />
          <span className="min-w-0 break-words">
            {approval.groupName} / {approval.environmentName} /{" "}
            <span className="font-medium">{approval.connectionName}</span>
          </span>
        </dd>
        <dt className="text-(--text2)">Tool</dt>
        <dd className="font-mono text-xs leading-5">{approval.tool}</dd>
        {/* UI review 2026-09-21 — the title says "Claude (agent mode)";
            this is the identifier that claim rests on. A client names
            itself in the MCP handshake, so the raw string is what the
            decision should be made on. */}
        {identity ? (
          <>
            <dt className="text-(--text2)">Agent</dt>
            <dd
              className="min-w-0 font-mono text-xs leading-5 break-all"
              data-testid="mcp-approval-agent"
            >
              {identity}
            </dd>
          </>
        ) : null}
      </dl>

      <TargetPreviewView preview={preview} argumentsJson={approval.argumentsJson} />

      {lock === "locked" ? (
        <p
          className="rounded-card border border-(--state-delayed)/40 bg-(--state-delayed)/10 px-3 py-2 text-xs"
          data-testid="mcp-approval-protected"
        >
          {approval.connectionName} is protected. Approving runs this call once; the connection
          stays read-only for everything else.
        </p>
      ) : null}

      <details className="text-sm" data-testid="mcp-approval-call">
        <summary className="cursor-pointer text-(--text2)">The exact call</summary>
        <CodeBlock label="The exact call" className="mt-2">
          {`${approval.tool}(${call})`}
        </CodeBlock>
      </details>
    </div>
  )
}

function TargetPreviewView({
  preview,
  argumentsJson,
}: {
  preview: TargetPreview
  argumentsJson: string
}) {
  if (preview?.kind === "job") {
    return (
      <div
        className="min-w-0 rounded-md border border-(--line) p-3 text-sm"
        data-testid="mcp-approval-preview"
      >
        <p className="font-medium break-words">{preview.name ?? "(unnamed job)"}</p>
        <p className="font-mono text-xs text-(--text2)">
          {preview.queue} #{preview.jobId}
          {preview.state ? ` · ${preview.state}` : ""}
          {typeof preview.attemptsMade === "number" ? ` · ${preview.attemptsMade} attempt(s)` : ""}
        </p>
        {preview.failedReason ? (
          <p className="mt-2 whitespace-pre-wrap break-words text-xs text-(--state-failed-text)">
            {preview.failedReason}
          </p>
        ) : null}
      </div>
    )
  }
  if (preview?.kind === "newJob") {
    const options = Object.entries(preview.options ?? {})
    return (
      <div
        className="flex min-w-0 flex-col gap-2 rounded-md border border-(--line) p-3 text-sm"
        data-testid="mcp-approval-preview"
      >
        <div className="min-w-0">
          <p className="font-medium break-words">{preview.name ?? "(unnamed job)"}</p>
          <p className="font-mono text-xs break-words text-(--text2)">
            new job in {preview.queue}
            {options.length === 0
              ? " · default options"
              : options.map(([key, value]) => ` · ${key}: ${String(value)}`).join("")}
          </p>
        </div>
        <JsonViewer value={preview.data ?? null} copyLabel="payload" collapseDeeperThan={3} />
      </div>
    )
  }
  if (preview?.kind === "schedulerChange" || preview?.kind === "schedulerRemoval") {
    const timing = describeSchedulerTiming(preview)
    return (
      <div
        className="flex flex-col gap-2 rounded-md border border-(--line) p-3 text-sm"
        data-testid="mcp-approval-preview"
      >
        <div>
          <p className="font-medium">
            {preview.schedulerId ?? "(no id)"}
            {preview.jobName ? (
              <span className="font-normal text-(--text2)"> → {preview.jobName}</span>
            ) : null}
          </p>
          <p className="font-mono text-xs text-(--text2)">
            {preview.kind === "schedulerRemoval" ? "scheduler of " : "scheduler in "}
            {preview.queue}
            {timing ? ` · ${timing}` : ""}
          </p>
          {typeof preview.nextRunMs === "number" ? (
            <p className="text-xs text-(--text2)">
              {preview.kind === "schedulerRemoval" ? "Would have run " : "Next run "}
              {new Date(preview.nextRunMs).toLocaleString()}
            </p>
          ) : null}
        </div>
        {preview.kind === "schedulerChange" ? (
          <JsonViewer value={preview.data ?? {}} copyLabel="template" collapseDeeperThan={3} />
        ) : null}
      </div>
    )
  }
  if (preview?.kind === "maintenance") {
    return (
      <div
        className="rounded-md border border-(--line) p-3 text-sm"
        data-testid="mcp-approval-preview"
      >
        <p className="font-medium">
          {preview.count.toLocaleString()} job{preview.count === 1 ? "" : "s"}
          {preview.queue ? ` in ${preview.queue}` : ""}
        </p>
        <p className="text-xs text-(--text2)">
          {describeMaintenanceScope(preview)}
          {preview.sparedSchedulerJobs
            ? ` · ${preview.sparedSchedulerJobs} scheduler run(s) left in place`
            : ""}
        </p>
      </div>
    )
  }
  if (preview?.kind === "bulk") {
    const names = preview.sampleNames ?? []
    return (
      <div
        className="rounded-md border border-(--line) p-3 text-sm"
        data-testid="mcp-approval-preview"
      >
        <p className="font-medium">
          {preview.count} job{preview.count === 1 ? "" : "s"}
          {preview.queue ? ` in ${preview.queue}` : ""}
        </p>
        {names.length > 0 ? (
          <ul className="mt-1 list-inside list-disc text-xs text-(--text2)">
            {names.map((name, i) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: names can repeat; order is stable
              <li key={`${name}-${i}`}>{name}</li>
            ))}
            {preview.count > names.length ? (
              <li>…and {preview.count - names.length} more</li>
            ) : null}
          </ul>
        ) : null}
      </div>
    )
  }
  // No structured preview: show what the agent sent, as data.
  let value: unknown = argumentsJson
  try {
    value = preview?.kind === "raw" ? preview.value : JSON.parse(argumentsJson)
  } catch {
    // keep the raw string
  }
  return (
    <div data-testid="mcp-approval-preview">
      <JsonViewer value={value} copyLabel="request" collapseDeeperThan={3} />
    </div>
  )
}
