import { ExternalLinkIcon, XIcon } from "lucide-react"
import { useState } from "react"

import { Button } from "@/components/ui/button"
import type { QueueRef } from "@/lib/tauri/bindings"
import { jobRoute, openRouteOrSay } from "@/lib/windows/openRoute"

import { type DrillDownTab, JobDrillDown } from "./JobDrillDown"

/**
 * A job beside its list: the job's drill-down in the
 * screen's inspector, with a way out to its own window and a close button.
 * The job page (`/…/$jobId`) still shows the job alone, for deep links and
 * Open in New Window.
 */
export function JobInspector({
  connectionId,
  queueRef,
  jobId,
  onClose,
}: {
  connectionId: string
  queueRef: QueueRef
  jobId: string
  onClose: () => void
}) {
  const [tab, setTab] = useState<DrillDownTab | undefined>(undefined)
  return (
    <div className="flex min-h-0 flex-col" data-testid="job-inspector">
      <div className="sticky top-0 z-10 flex items-center justify-end gap-1 border-b border-(--line) bg-(--bg) px-2 py-1">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Open in New Window"
          title="Open in New Window (⌥↩)"
          onClick={() =>
            openRouteOrSay(jobRoute(connectionId, queueRef.name, jobId), { tab: false })
          }
        >
          <ExternalLinkIcon aria-hidden="true" />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Close the inspector"
          onClick={onClose}
          data-testid="job-inspector-close"
        >
          <XIcon aria-hidden="true" />
        </Button>
      </div>
      <div className="flex flex-col gap-4 p-4">
        <JobDrillDown
          // A new job starts on its default tab.
          key={`${queueRef.name}:${jobId}`}
          connectionId={connectionId}
          queueRef={queueRef}
          jobId={jobId}
          tab={tab}
          onTabChange={setTab}
          onRemoved={onClose}
        />
      </div>
    </div>
  )
}
