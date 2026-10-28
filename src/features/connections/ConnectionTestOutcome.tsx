import { CircleCheckIcon, LoaderCircleIcon, TriangleAlertIcon } from "lucide-react"
import type { ReactNode } from "react"

import { formatCount } from "@/domain/format"
import type { ConnectionProbe, TestConnectionResult } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { plainTestError } from "./plainError"

/** Step 2's head: testing, what the test found, or why it failed. */
export function TestOutcome({
  testing,
  result,
  target,
  tls,
}: {
  testing: boolean
  result: TestConnectionResult | null
  target: { host: string; port: number }
  tls: boolean
}) {
  if (testing || !result) {
    return (
      <p className="flex items-center gap-2 text-sm text-(--text2)" data-testid="test-running">
        <LoaderCircleIcon className="size-4 animate-spin" aria-hidden="true" />
        Testing {target.host}:{target.port}
      </p>
    )
  }
  if (!result.success) {
    const { headline, detail } = plainTestError(result.error, target)
    return (
      <div
        data-testid="test-result"
        data-success="false"
        className="flex items-start gap-2 rounded-md border border-(--danger)/40 bg-(--danger)/10 px-3 py-2 text-sm"
      >
        <TriangleAlertIcon
          className="mt-0.5 size-4 shrink-0 text-(--state-failed-text)"
          aria-hidden="true"
        />
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <p className="font-medium break-words text-(--state-failed-text)">{headline}</p>
          {detail ? <p className="font-mono text-xs break-words text-(--text2)">{detail}</p> : null}
          <p className="text-xs text-(--text2)">Back keeps what you typed so you can fix it.</p>
        </div>
      </div>
    )
  }
  return (
    <div
      data-testid="test-result"
      data-success="true"
      className="flex flex-col gap-2 rounded-md border border-(--line) px-3 py-2 text-sm"
    >
      <p className="flex items-center gap-2 font-medium">
        <CircleCheckIcon className="size-4 shrink-0 text-(--state-completed)" aria-hidden="true" />
        Connected to {target.host}:{target.port}
      </p>
      {result.sshHostKeyPinned?.wasNewPin ? (
        <p className="text-xs wrap-anywhere text-(--text2)" data-testid="ssh-hostkey-pinned-banner">
          First connect to the bastion: its host key is now pinned (
          <span className="font-mono">{result.sshHostKeyPinned.fingerprint}</span>).
        </p>
      ) : null}
      <ProbeFacts probe={result.probe ?? null} version={result.redisVersion} tls={tls} />
    </div>
  )
}

function ProbeFacts({
  probe,
  version,
  tls,
}: {
  probe: ConnectionProbe | null
  version: string | null
  tls: boolean
}) {
  const facts: Array<[string, ReactNode]> = [
    ["Latency", probe ? `${probe.latencyMs} ms` : "-"],
    ["Redis", version ? `v${version}` : "version not reported"],
    ["TLS", tls ? "on" : "off"],
  ]
  if (probe) {
    const more = probe.queuesFound - probe.queueSample.length
    facts.push(
      [
        "Queues",
        probe.queuesFound === 0 ? (
          "none found yet"
        ) : (
          <span key="queues" className="flex flex-col">
            <span>
              {formatCount(probe.queuesFound)}
              {probe.scanCapped ? " or more" : ""}
            </span>
            <span className="font-mono text-xs wrap-anywhere text-(--text2)">
              {probe.queueSample.join(", ")}
              {more > 0 ? ` and ${formatCount(more)} more` : ""}
            </span>
          </span>
        ),
      ],
      ["Schedulers", formatCount(probe.schedulers)],
      [
        "Workers",
        probe.workers === null ? "not listed by this server" : formatCount(probe.workers),
      ],
    )
  }
  return (
    <dl
      className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm"
      data-testid="probe-facts"
    >
      {facts.map(([label, value]) => (
        <div key={label} className="contents">
          <dt className="text-(--text2)">{label}</dt>
          <dd className={cn("min-w-0 font-tabular break-words")}>{value}</dd>
        </div>
      ))}
    </dl>
  )
}
