import { ArrowUpToLineIcon, type LucideIcon, RotateCcwIcon, Trash2Icon } from "lucide-react"
import { useCallback, useMemo, useState } from "react"
import { useCapabilities } from "@/capabilities"
import { displayChord } from "@/commands/chords"
import { useCommandHandler } from "@/commands/handlers"
import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { Kbd } from "@/components/data/Kbd"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { usePromoteJob, useRemoveJob, useRetryJob } from "@/data/jobs/mutations"
import { withOneShotUnlock } from "@/data/safety/oneShotUnlock"
import { useConnectionLockState } from "@/data/safety/queries"
import type { Job, QueueId } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

/**
 * single-job action affordances (retry / remove / promote).
 *
 * Two visual variants behind one component:
 *
 *   - **Full** (`compact={false}`, default): labeled buttons, each
 *     with a kbd hint, mounted on the job drill-down header, only the
 *     ones that apply to the job's state (a greyed-out "Promote" on a
 *     failed job is noise): the state's own action first, as the primary
 *     button, Remove last. Has keyboard shortcuts (registry commands):
 *     ⇧⌘R retry, ⌘⌫ remove, ⌥⌘P promote (ctrl-equivalents on
 *     non-Mac). Shortcuts are `e.preventDefault()`-ed and only fire when
 *     the action applies (no silent confusion).
 *
 *   - **Compact** (`compact={true}`): icon-only, mounted on JobList
 *     rows. Only renders the action(s) relevant to the row's state
 *     (`failed` → Retry, `delayed` → Promote). No keyboard shortcuts
 *     (the row component has its own hover context, so multi-row
 *     keystroke ambiguity is avoided; bulk select owns row-level
 *     keyboard handling).
 *
 * State-precondition enablement:
 *   - Retry: enabled when `job.state === "failed"`.
 *   - Promote: enabled when `job.state === "delayed"`.
 *   - Remove: always enabled (the job hash exists by definition) and
 *     always **confirmed** (button and `cmd-Backspace` alike): it deletes
 *     the job from Redis for good, one click was too close to a mistake.
 *     The dialog opens with focus on Cancel.
 *
 * Locked-connection rejection is handled by the toast on
 * error; buttons stay enabled regardless of lock state, by design
 * ("the user discovers the lock at the moment of action").
 *
 * A backend that does not declare single-job actions gets none: no
 * buttons, no menu commands.
 */

export interface JobActionsProps {
  connectionId: string
  queueId: QueueId
  job: Job
  compact?: boolean
  enableShortcuts?: boolean
  /** The job is gone: the page showing it has nothing left to show. */
  onRemoved?: () => void
}

export function JobActions({
  connectionId,
  queueId,
  job,
  compact = false,
  enableShortcuts,
  onRemoved,
}: JobActionsProps) {
  const retry = useRetryJob()
  const remove = useRemoveJob()
  const promote = usePromoteJob()

  // Shortcuts default to ON for the full variant (drill-down header),
  // OFF for the compact variant (list rows: avoids row-hover keystroke
  // ambiguity, since bulk-select owns the keyboard).
  const shortcutsActive = enableShortcuts ?? !compact

  const single = useCapabilities(connectionId).can("jobActions.single")
  const canRetry = single && job.state === "failed"
  const canPromote = single && job.state === "delayed"

  const input = useMemo(
    () => ({ connectionId, queueId, jobId: job.jobRef.id }),
    [connectionId, queueId, job.jobRef.id],
  )

  // Memoize handlers so the keyboard hook's dep array stays stable
  // across renders. The hook re-binds the listener when handler
  // changes.
  const handleRetry = useCallback(() => {
    if (canRetry && !retry.isPending) retry.mutate(input)
  }, [canRetry, retry, input])
  const [confirmingRemove, setConfirmingRemove] = useState(false)
  // Asks first; `confirmRemove` is what the dialog's button runs.
  const handleRemove = useCallback(() => {
    if (!remove.isPending) setConfirmingRemove(true)
  }, [remove.isPending])
  // A protected connection asks for the queue's name, and the
  // removal runs under a one-shot, audited unlock.
  const { data: lockState } = useConnectionLockState(connectionId)
  const locked = lockState === "locked"
  const confirmRemove = useCallback(() => {
    if (remove.isPending) return
    const send = () => remove.mutateAsync(input)
    ;(locked ? withOneShotUnlock(connectionId, send) : send()).then(
      () => {
        setConfirmingRemove(false)
        onRemoved?.()
      },
      // The toast says why; the sheet has nothing more to offer.
      () => setConfirmingRemove(false),
    )
  }, [remove, input, onRemoved, locked, connectionId])
  const handlePromote = useCallback(() => {
    if (canPromote && !promote.isPending) promote.mutate(input)
  }, [canPromote, promote, input])

  // Job › Retry (⇧⌘R), Promote (⌥⌘P), Remove… (⌘⌫): registry
  // commands while this job's actions are the active ones (the job page).
  useCommandHandler("job.retry", handleRetry, {
    enabled: shortcutsActive && canRetry && !retry.isPending,
    priority: 1,
  })
  useCommandHandler("job.promote", handlePromote, {
    enabled: shortcutsActive && canPromote && !promote.isPending,
    priority: 1,
  })
  useCommandHandler("job.remove", handleRemove, {
    enabled: shortcutsActive && single && !remove.isPending,
    priority: 1,
  })

  if (!single) return null

  if (compact) {
    return (
      <div className="flex items-center gap-1" data-testid="job-actions-compact">
        {canRetry ? (
          <CompactIconButton
            icon={RotateCcwIcon}
            label="Retry"
            disabled={retry.isPending}
            onClick={(e) => {
              e.preventDefault()
              e.stopPropagation()
              handleRetry()
            }}
            testId="job-actions-retry"
          />
        ) : null}
        {canPromote ? (
          <CompactIconButton
            icon={ArrowUpToLineIcon}
            label="Promote"
            disabled={promote.isPending}
            onClick={(e) => {
              e.preventDefault()
              e.stopPropagation()
              handlePromote()
            }}
            testId="job-actions-promote"
          />
        ) : null}
      </div>
    )
  }

  // Full variant: drill-down header.
  return (
    // `contents`: the buttons join the header's own wrapping row.
    <div className="contents" data-testid="job-actions">
      {canRetry ? (
        <ActionButton
          label="Retry"
          icon={RotateCcwIcon}
          kbdHint={displayChord("cmd+shift+r")}
          disabled={retry.isPending}
          onClick={handleRetry}
          testId="job-actions-retry"
          variant="primary"
          hint="Put this job back in the queue; a worker runs it again"
        />
      ) : null}
      {canPromote ? (
        <ActionButton
          label="Promote"
          icon={ArrowUpToLineIcon}
          kbdHint={displayChord("alt+cmd+p")}
          disabled={promote.isPending}
          onClick={handlePromote}
          testId="job-actions-promote"
          variant="primary"
          hint="Run it now instead of waiting for its delay"
        />
      ) : null}
      <ActionButton
        label="Remove"
        icon={Trash2Icon}
        kbdHint={displayChord("cmd+backspace")}
        disabled={remove.isPending}
        onClick={handleRemove}
        testId="job-actions-remove"
        variant="danger"
        hint="Delete this job from Redis"
      />
      <ConfirmSheet
        open={confirmingRemove}
        onOpenChange={setConfirmingRemove}
        title="Remove this job?"
        description={
          job.state === "active"
            ? "A worker is processing it right now: it will fail when it tries to report back."
            : "This cannot be undone."
        }
        danger
        removed={[
          { label: `${job.name || "(unnamed job)"} #${job.jobRef.id}` },
          { label: "Its logs" },
        ]}
        kept={[{ label: `Every other job of ${job.jobRef.queueName}` }]}
        protectedName={locked ? job.jobRef.queueName : undefined}
        confirmLabel={remove.isPending ? "Removing" : "Remove 1 job"}
        onConfirm={confirmRemove}
        busy={remove.isPending}
        testId="job-remove-confirm"
        confirmTestId="job-remove-confirm-action"
      />
    </div>
  )
}

interface ActionButtonProps {
  label: string
  icon: LucideIcon
  kbdHint: string
  disabled: boolean
  onClick: () => void
  testId: string
  variant: "primary" | "danger"
  /** What the action does, in a tooltip. */
  hint: string
}

function ActionButton(props: ActionButtonProps) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            type="button"
            size="sm"
            variant={props.variant}
            disabled={props.disabled}
            onClick={props.onClick}
            data-testid={props.testId}
            className="gap-1.5"
          >
            <props.icon className="size-3.5" aria-hidden="true" />
            <span>{props.label}</span>
            {/* On a filled button the hint takes the label's colour. */}
            <Kbd className="hidden border-current/40 bg-transparent text-current sm:inline-flex">
              {props.kbdHint}
            </Kbd>
          </Button>
        }
      />
      <TooltipContent>{props.hint}</TooltipContent>
    </Tooltip>
  )
}

interface CompactIconButtonProps {
  icon: LucideIcon
  label: string
  disabled: boolean
  onClick: (e: React.MouseEvent) => void
  testId: string
}

function CompactIconButton(props: CompactIconButtonProps) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type="button"
            disabled={props.disabled}
            onClick={props.onClick}
            data-testid={props.testId}
            aria-label={props.label}
            className={cn(
              "flex size-7 items-center justify-center rounded border border-transparent text-(--text2) hover:border-(--line) hover:bg-(--surf) hover:text-(--text)",
              props.disabled && "opacity-50",
            )}
          >
            <props.icon size={14} />
          </button>
        }
      />
      <TooltipContent>{props.label}</TooltipContent>
    </Tooltip>
  )
}
