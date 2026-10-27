import { Skeleton } from "@/components/ui/skeleton"
import type { QueueRef } from "@/lib/tauri/bindings"

import { FlowGraph } from "./FlowGraph"
import { useFlow } from "./useFlow"

/** The Flow tab of a job: its whole flow, this job highlighted. */
export function JobFlowTab({
  connectionId,
  queueRef,
  jobId,
}: {
  connectionId: string
  queueRef: QueueRef
  jobId: string
}) {
  const { data, isLoading, isError, error } = useFlow({ connectionId, queueRef, jobId })
  if (isLoading) return <Skeleton className="h-40 w-full" />
  if (isError || !data) {
    return (
      <p role="alert" className="text-sm text-(--state-failed-text)">
        Could not read the flow: {error instanceof Error ? error.message : "unknown error"}
      </p>
    )
  }
  if (data.nodes.length <= 1) {
    return (
      <p className="text-sm text-(--text2)" data-testid="job-flow-none">
        This job is not part of a flow.
      </p>
    )
  }
  return <FlowGraph connectionId={connectionId} tree={data} />
}
