import { Link } from "@tanstack/react-router"
import { PauseIcon, PlayIcon } from "lucide-react"

import { useCapabilities } from "@/capabilities"
import { displayChord } from "@/commands/chords"
import { runCommand, useCommandHandler } from "@/commands/handlers"
import { EnvDot } from "@/components/data/Env"
import { Kbd } from "@/components/data/Kbd"
import { Toolbar } from "@/components/data/Toolbar"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { usePauseQueue, useResumeQueue } from "@/data/queues/mutations"
import { LockButton } from "@/features/safety"
import { RefreshControl } from "@/features/settings"
import type { Connection, Queue, QueueId } from "@/lib/tauri/bindings"

import { QueueMaintenanceMenu } from "./QueueMaintenanceMenu"

/**
 * The queue screen's toolbar: back to the connection, the queue's key and
 * warnings (paused, no worker), and the actions its backend declares. It is
 * there from the start: it does not wait for the queue list to arrive.
 */
export function QueueToolbar({
  connectionId,
  queueName,
  queueId,
  queue,
  connection,
  workerCount,
  locked,
  onAddJob,
  onShowWorkers,
}: {
  connectionId: string
  queueName: string
  queueId: QueueId
  /** `undefined` until the queue list has arrived. */
  queue: Queue | undefined
  connection: Connection | undefined
  /** `undefined` while unknown, or when the server does not list clients. */
  workerCount: number | undefined
  locked: boolean
  onAddJob: () => void
  onShowWorkers: () => void
}) {
  const { can } = useCapabilities(connectionId)
  const maintenance =
    can("maintenance.retryAllFailed") || can("maintenance.clean") || can("maintenance.drain")
  return (
    <Toolbar
      leading={
        <Link
          to="/connections/$connectionId"
          params={{ connectionId }}
          className="flex items-center gap-1.5 text-sm text-(--text2) hover:text-(--text) hover:underline"
          title="Back to the connection"
        >
          ←{" "}
          {connection ? (
            <>
              <EnvDot color={connection.envColor} label={connection.envLabel} />
              {connection.name}
            </>
          ) : (
            "Queues"
          )}
        </Link>
      }
      title={queueName}
      meta={
        <>
          <span className="hidden font-mono text-xs text-(--text2) lg:inline">{queueId}</span>
          {queue?.isPaused ? (
            <span className="rounded-tag bg-(--state-delayed)/15 px-1.5 py-px text-xs text-(--state-delayed-text)">
              paused
            </span>
          ) : null}
          {workerCount === 0 ? (
            <button
              type="button"
              onClick={onShowWorkers}
              title="No BullMQ worker is connected to this queue: waiting jobs will not be processed."
              data-testid="queue-no-worker"
              className="rounded-tag bg-(--state-delayed)/15 px-1.5 py-px text-xs text-(--state-delayed-text) hover:bg-(--state-delayed)/25"
            >
              no worker
            </button>
          ) : null}
        </>
      }
      actions={
        <>
          <LockButton connectionId={connectionId} />
          <RefreshControl connectionId={connectionId} />
          {/* Pause / Resume, disabled while the connection is locked.
              The toolbar is there from the start (it does not wait for
              the queue list to arrive); Pause waits for the
              queue's state, the others only need its name. */}
          {can("queueControl") ? (
            <QueuePauseButton
              connectionId={connectionId}
              queueId={queueId}
              queueName={queueName}
              isPaused={queue?.isPaused ?? false}
              loading={!queue}
              locked={locked}
            />
          ) : null}
          {maintenance ? (
            <span>
              <QueueMaintenanceMenu connectionId={connectionId} queueId={queueId} locked={locked} />
            </span>
          ) : null}
          {can("jobActions.add") ? (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger
                  render={
                    <span className="inline-block">
                      <Button
                        variant="primary"
                        type="button"
                        size="sm"
                        onClick={onAddJob}
                        disabled={locked}
                        data-testid="queue-add-job"
                      >
                        Add job
                      </Button>
                    </span>
                  }
                />
                <TooltipContent>
                  {locked
                    ? "This connection is read-only. Unlock it from the toolbar (⇧⌘U) to add jobs."
                    : "Enqueue a job; workers process it like any other"}
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          ) : null}
        </>
      }
    />
  )
}

/**
 * Pause / Resume toggle for the queue header. Reads
 * `isPaused` from the cached queue counter (polled every
 * 1s) and flips the button label between Pause and Resume. The
 * `cmd-shift-P` shortcut wires the same toggle from the keyboard.
 *
 * The label flips at once (optimistic, see `usePauseQueue`)
 * and the button stays busy until the refreshed queue list confirms it;
 * disabled until the queue's state is known.
 */
function QueuePauseButton({
  connectionId,
  queueId,
  queueName,
  isPaused,
  loading,
  locked,
}: {
  connectionId: string
  queueId: QueueId
  queueName: string
  isPaused: boolean
  /** The queue's state is not known yet. */
  loading: boolean
  locked: boolean
}) {
  const pauseMut = usePauseQueue()
  const resumeMut = useResumeQueue()
  const busy = pauseMut.isPending || resumeMut.isPending || loading

  // Queue › Pause / Resume (⇧⌘P), a registry command.
  useCommandHandler(
    "queue.pause",
    () => {
      if (isPaused) resumeMut.mutate({ connectionId, queueId })
      else pauseMut.mutate({ connectionId, queueId })
    },
    {
      // Locked: the lock says how to unlock: no refused write, no error toast.
      enabled: !busy && !locked,
      label: `${isPaused ? "Resume" : "Pause"} “${queueName}”`,
    },
  )

  return (
    <Button
      type="button"
      size="sm"
      variant="default"
      onClick={() => runCommand("queue.pause")}
      disabled={busy || locked}
      title={locked ? "This connection is read-only. Unlock it from the toolbar (⇧⌘U)." : undefined}
      data-testid="queue-pause-toggle"
      className="gap-1.5"
    >
      {isPaused ? <PlayIcon aria-hidden="true" /> : <PauseIcon aria-hidden="true" />}
      <span>{isPaused ? "Resume" : "Pause"}</span>
      <Kbd className="hidden xl:inline-flex">{displayChord("cmd+shift+p")}</Kbd>
    </Button>
  )
}
