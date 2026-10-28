import { useQueries, useQueryClient } from "@tanstack/react-query"
import { Link } from "@tanstack/react-router"
import { TriangleAlertIcon, XIcon } from "lucide-react"
import { useMemo, useState } from "react"
import { toast } from "sonner"
import { useJobStates } from "@/capabilities"
import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { StackedBar } from "@/components/data/charts"
import { SectionHeader } from "@/components/data/SectionHeader"
import { StatePill } from "@/components/data/StatePill"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { flowKeys } from "@/data/flows/keys"
import { fetchFlowRoots, useFlow } from "@/data/flows/queries"
import { removeJob, retryJob } from "@/data/jobs/mutations"
import { useQueues } from "@/data/queues/queries"
import { unwrap } from "@/data/result"
import { withOneShotUnlock } from "@/data/safety/oneShotUnlock"
import { useConnectionLockState } from "@/data/safety/queries"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { formatDurationMs } from "@/domain/format"
import { formatAge } from "@/domain/formatAge"
import { useSharedPolls } from "@/lib/polling/sharedPoll"
import type { FlowNode, FlowRoot } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

export const flowId = (root: Pick<FlowRoot, "jobRef">) => `${root.jobRef.queueId}/${root.jobRef.id}`

/** Flow roots are a few round trips per queue: never faster than this. */
const FLOWS_REFRESH_FLOOR_MS = 15_000

/** Every queue's flow roots (`list_flow_roots`), newest first. */
export function useConnectionFlows(connectionId: string) {
  const queues = useQueues(connectionId).data
  // The refresh setting (15 s floor), one self-paced poller per
  // query (see `useSharedPoll`).
  const interval = useRefetchInterval(FLOWS_REFRESH_FLOOR_MS)
  useSharedPolls(
    (queues ?? []).map((q) => flowKeys.roots(connectionId, q.id)),
    interval,
  )
  const lists = useQueries({
    queries: (queues ?? []).map((q) => ({
      queryKey: flowKeys.roots(connectionId, q.id),
      queryFn: () => fetchFlowRoots(connectionId, q.id),
      staleTime: 10_000,
      refetchInterval: false as const,
    })),
  })
  const version = lists.map((l) => l.dataUpdatedAt).join(",")
  // biome-ignore lint/correctness/useExhaustiveDependencies: `version` stands for `lists`.
  const roots = useMemo(
    () =>
      lists
        .flatMap((l) => l.data?.roots ?? [])
        .sort(
          (a, b) =>
            Number(b.blocked) - Number(a.blocked) || (b.timestamp ?? 0) - (a.timestamp ?? 0),
        ),
    [version],
  )
  return {
    roots,
    loading: !queues || lists.some((l) => l.isLoading),
    capped: lists.some((l) => l.data?.capped),
  }
}

/**
 * The connection's recent flows: each with how its
 * children are doing as a bar, and its state; a flow blocked behind a failed
 * child comes first. A flow opens beside the list as its tree.
 */
export function ConnectionFlows({
  connectionId,
  selected,
  onSelect,
}: {
  connectionId: string
  selected: string | null
  onSelect: (id: string) => void
}) {
  const { roots, loading, capped } = useConnectionFlows(connectionId)
  const states = useJobStates(connectionId)
  const [now] = useState(() => Date.now())
  if (loading) {
    return (
      <div className="flex flex-col gap-2" role="status" aria-label="Loading flows">
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} className="h-10 w-full" />
        ))}
      </div>
    )
  }
  if (roots.length === 0) {
    return (
      <p className="rounded-card border border-(--line) p-6 text-center text-sm text-(--text2)">
        No flow among the recent jobs of this connection.
      </p>
    )
  }
  return (
    <section aria-label="Flows" className="flex flex-col gap-2" data-testid="connection-flows">
      <SectionHeader title="Recent flows" count={roots.length} />
      <ul className="divide-y divide-(--line) rounded-card border border-(--line)">
        {roots.map((r) => {
          const id = flowId(r)
          return (
            <li key={id}>
              <button
                type="button"
                onClick={() => onSelect(id)}
                aria-pressed={selected === id}
                className={cn(
                  "grid w-full grid-cols-[minmax(8rem,1.4fr)_minmax(8rem,1fr)_auto_auto] items-center gap-3 px-3 py-2 text-left text-sm hover:bg-(--surf2) focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-(--accent)/60",
                  selected === id && "bg-(--sel)",
                )}
                data-testid="flow-row"
                data-blocked={r.blocked ? "true" : undefined}
              >
                <span className="flex min-w-0 flex-col">
                  <span className="font-medium break-words">{r.name || "(unnamed)"}</span>
                  <span className="font-tabular text-xs wrap-anywhere text-(--text2)">
                    {r.jobRef.queueName}/{r.jobRef.id}
                    {r.timestamp ? ` · ${formatAge(Math.max(now - r.timestamp, 0))} ago` : ""}
                  </span>
                </span>
                <span className="flex min-w-0 flex-col gap-1">
                  <StackedBar
                    label={`${r.childrenTotal} children`}
                    segments={[
                      {
                        key: "completed",
                        label: "Completed",
                        value: r.childrenCompleted,
                        color: "var(--state-completed)",
                      },
                      {
                        key: "failed",
                        label: "Failed",
                        value: r.childrenFailed,
                        color: "var(--state-failed)",
                      },
                      {
                        key: "pending",
                        label: "Pending",
                        value: r.childrenPending,
                        color: "var(--state-waiting)",
                      },
                    ]}
                  />
                  <span className="text-2xs text-(--text2)">
                    {r.childrenCompleted}/{r.childrenTotal} done
                    {r.childrenFailed > 0 ? ` · ${r.childrenFailed} failed` : ""}
                  </span>
                </span>
                <StatePill state={r.state} states={states} />
                <span
                  className={cn(
                    "text-xs",
                    r.blocked ? "text-(--state-failed-text)" : "text-(--text2)",
                  )}
                >
                  {r.blocked ? "Blocked" : ""}
                </span>
              </button>
            </li>
          )
        })}
      </ul>
      {capped ? (
        <p className="text-xs text-(--text2)">
          Older flows are not listed: Ranch looks at the recent jobs only.
        </p>
      ) : null}
    </section>
  )
}

/**
 * A flow beside the list: the parent → children tree with each job's
 * state and duration, what finished children returned, the failed child's
 * error; when the parent waits on a child that failed for good, a banner
 * explains it and offers Retry child / Remove child (in sheets).
 */
export function FlowInspector({
  connectionId,
  id,
  onClose,
}: {
  connectionId: string
  id: string
  onClose: () => void
}) {
  // The flow is read from its root as the list gave it: the id is only
  // compared, never taken apart.
  const roots = useConnectionFlows(connectionId).roots
  const root = roots.find((r) => flowId(r) === id)
  const flow = useFlow(
    { connectionId, queueId: root?.jobRef.queueId ?? "", jobId: root?.jobRef.id ?? "" },
    root !== undefined,
  )
  const tree = flow.data
  const children = useMemo(() => {
    const out = new Map<string, FlowNode[]>()
    for (const n of tree?.nodes ?? []) {
      if (n.parentKey) out.set(n.parentKey, [...(out.get(n.parentKey) ?? []), n])
    }
    return out
  }, [tree])
  const top = tree?.nodes.find((n) => n.key === tree.rootKey)
  const blocking = (tree?.nodes ?? []).filter((n) => n.blocksParent)

  return (
    <div className="flex min-h-0 flex-col" data-testid="flow-inspector">
      <div className="flex items-center justify-end border-b border-(--line) px-2 py-1">
        <Button variant="ghost" size="icon-sm" aria-label="Close the inspector" onClick={onClose}>
          <XIcon aria-hidden="true" />
        </Button>
      </div>
      <div className="flex flex-col gap-4 p-4">
        {flow.isLoading ? <Skeleton className="h-24 w-full" /> : null}
        {flow.error ? (
          <p role="alert" className="text-sm text-(--state-failed-text)">
            {flow.error instanceof Error ? flow.error.message : String(flow.error)}
          </p>
        ) : null}
        {tree?.blocked
          ? blocking.map((child) => (
              <BlockedBanner key={child.key} connectionId={connectionId} child={child} />
            ))
          : null}
        {top ? (
          <ol className="flex flex-col gap-1" data-testid="flow-tree">
            <TreeNode connectionId={connectionId} node={top} childrenOf={children} depth={0} />
          </ol>
        ) : null}
        {tree?.truncated ? (
          <p className="text-xs text-(--text2)">The flow holds more jobs than are shown.</p>
        ) : null}
      </div>
    </div>
  )
}

function TreeNode({
  connectionId,
  node,
  childrenOf,
  depth,
}: {
  connectionId: string
  node: FlowNode
  childrenOf: Map<string, FlowNode[]>
  depth: number
}) {
  const kids = childrenOf.get(node.key) ?? []
  const states = useJobStates(connectionId)
  return (
    <li className="flex flex-col gap-1" data-testid="flow-node" data-key={node.key}>
      <div
        className={cn(
          "flex flex-col gap-1 rounded-card border px-3 py-2 text-sm",
          node.blocksParent ? "border-(--danger)/50 bg-(--danger)/5" : "border-(--line)",
        )}
        style={{ marginLeft: `${depth * 1.25}rem` }}
      >
        <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span className="font-medium break-words">
            {node.missing ? "(removed)" : node.name || "(unnamed)"}
          </span>
          <StatePill state={node.state} states={states} />
          {node.durationMs !== null ? (
            <span className="font-tabular text-xs text-(--text2)">
              {formatDurationMs(node.durationMs)}
            </span>
          ) : null}
        </span>
        <Link
          to="/connections/$connectionId/queues/$queueId"
          params={{ connectionId, queueId: node.jobRef.queueId }}
          search={{ job: node.jobRef.id }}
          className="self-start font-tabular text-xs wrap-anywhere text-(--text2) hover:underline"
        >
          {node.jobRef.queueName}/{node.jobRef.id}
        </Link>
        {node.failedReason ? (
          <span className="font-tabular text-xs break-words text-(--state-failed-text)">
            {node.failedReason.split("\n")[0]}
          </span>
        ) : null}
        {node.returnValue ? (
          <span
            className="font-tabular text-xs wrap-anywhere text-(--text2)"
            data-testid="flow-node-returned"
          >
            returned {node.returnValue}
          </span>
        ) : null}
      </div>
      {kids.length > 0 ? (
        <ol className="flex flex-col gap-1">
          {kids.map((k) => (
            <TreeNode
              key={k.key}
              connectionId={connectionId}
              node={k}
              childrenOf={childrenOf}
              depth={depth + 1}
            />
          ))}
        </ol>
      ) : null}
    </li>
  )
}

function BlockedBanner({ connectionId, child }: { connectionId: string; child: FlowNode }) {
  const qc = useQueryClient()
  const { data: lockState } = useConnectionLockState(connectionId)
  const locked = lockState === "locked"
  const [action, setAction] = useState<"retry" | "remove" | null>(null)
  const [busy, setBusy] = useState(false)
  const input = { connectionId, queueId: child.jobRef.queueId, jobId: child.jobRef.id }

  const run = async () => {
    if (!action) return
    setBusy(true)
    try {
      const send = () => (action === "retry" ? retryJob(input) : removeJob(input))
      unwrap(await (locked ? withOneShotUnlock(connectionId, send) : send()))
      toast.success(action === "retry" ? `${child.name} retried` : `${child.name} removed`)
      setAction(null)
      qc.invalidateQueries({ queryKey: flowKeys.trees(connectionId) })
      qc.invalidateQueries({ queryKey: flowKeys.allRoots(connectionId) })
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div
      role="alert"
      className="flex flex-col gap-2 rounded-card border border-(--danger)/40 bg-(--danger)/10 p-3 text-sm"
      data-testid="flow-blocked"
    >
      <p className="flex items-start gap-2">
        <TriangleAlertIcon
          className="mt-0.5 size-4 shrink-0 text-(--state-failed-text)"
          aria-hidden="true"
        />
        <span>
          <span className="font-medium">{child.name || "A child"}</span> failed for good and its
          parent waits for it: nothing in its options lets the parent go on. Retry the child, or
          remove it so the parent can carry on.
        </span>
      </p>
      <div className="flex gap-2">
        <Button
          size="sm"
          variant="primary"
          onClick={() => setAction("retry")}
          data-testid="flow-retry-child"
        >
          Retry child
        </Button>
        <Button
          size="sm"
          variant="danger"
          onClick={() => setAction("remove")}
          data-testid="flow-remove-child"
        >
          Remove child
        </Button>
      </div>
      <ConfirmSheet
        open={action !== null}
        onOpenChange={(open) => {
          if (!open) setAction(null)
        }}
        title={action === "remove" ? `Remove ${child.name}?` : `Retry ${child.name}?`}
        description={
          action === "remove"
            ? "The parent stops waiting for it and goes on with the other children."
            : "It goes back to waiting; the parent goes on once it completes."
        }
        danger={action === "remove"}
        removed={
          action === "remove" ? [{ label: `${child.jobRef.queueName}/${child.jobRef.id}` }] : []
        }
        kept={action === "remove" ? [{ label: "The parent and the other children" }] : []}
        protectedName={locked ? child.jobRef.queueName : undefined}
        confirmLabel={action === "remove" ? "Remove child" : "Retry child"}
        onConfirm={() => void run()}
        busy={busy}
        testId="flow-child-sheet"
        confirmTestId="flow-child-confirm"
      />
    </div>
  )
}
