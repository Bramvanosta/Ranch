import { Skeleton } from "@/components/ui/skeleton"
import { useFlow } from "@/data/flows/queries"
import type { QueueId } from "@/lib/tauri/bindings"
import { FlowGraph } from "./FlowGraph"

/** The Flow tab of a job: its whole flow, this job highlighted. */
export function JobFlowTab({
  connectionId,
  queueId,
  jobId,
}: {
  connectionId: string
  queueId: QueueId
  jobId: string
}) {
  const { data, isLoading, isError, error } = useFlow({ connectionId, queueId, jobId })
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
