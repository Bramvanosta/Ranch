/**
 * vim-style keyboard navigation for job lists.
 *
 * Exposes a `focusedIndex` + setter pair, mounts global key bindings,
 * and triggers the right mutation on action chords.
 *
 * Bindings:
 *   - `j` / `↓`          → next row
 *   - `k` / `↑`          → previous row
 *   - `PageDown` / `PageUp` → ±10 rows
 *   - `Enter`            → drill into the focused job
 *   - `cmd-R`            → retry the focused job
 *   - `cmd-Backspace`    → select the focused job and open the bulk bar's
 *                          "Remove… Enter to execute, Esc to cancel" step,
 *                          never a removal on the spot (the first row, for
 *                          anyone who had not touched the arrows).
 *   - `cmd-P`            → promote the focused job
 *
 * With jobs selected, the three chords belong to the `BulkActionBar` (which
 * previews before it acts): this hook stays out of it, and does not act on
 * the focused row as well.
 *
 * Nothing is focused until a motion key is pressed: the first `j` / `↓`
 * marks the first row (it does not skip it), and `Enter` and the action
 * chords do nothing before that; acting on a row nobody can see marked was
 * how `cmd-R` retried "the first job" by surprise.
 *
 * Input-field guard: every binding no-ops when the active element is
 * an input / textarea / contenteditable (so typing in the filter
 * doesn't move the focused row or fire an action).
 */
import { useNavigate } from "@tanstack/react-router"
import { useCallback, useEffect, useRef, useState } from "react"
import { useCapabilities } from "@/capabilities"
import { useCommandHandler } from "@/commands/handlers"
import { usePromoteJob, useRetryJob } from "@/data/jobs/mutations"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import type { Job } from "@/lib/tauri/bindings"
import { jobRoute, openRouteOrSay } from "@/lib/windows/openRoute"
import { useJobSelectionStore } from "@/stores/useJobSelectionStore"

function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  const tag = target.tagName.toLowerCase()
  if (tag === "input" || tag === "textarea") return true
  if (target.isContentEditable) return true
  return false
}

export interface UseJobListNavigationOptions {
  jobs: Job[]
  connectionId: string
  /** Called when the focused-index changes, typically wired to the
   *  virtualizer's `scrollToIndex`. */
  onFocusChange?: (index: number) => void
  /** Enter opens the focused job here (the inspector) instead of on its page. */
  onOpen?: (job: Job) => void
}

export function useJobListNavigation({
  jobs,
  connectionId,
  onFocusChange,
  onOpen,
}: UseJobListNavigationOptions) {
  const [focusedIndex, setFocusedIndex] = useState(0)
  // Has the keyboard been used on this list yet?
  const engaged = useRef(false)
  // The same, as state: the first keystroke may leave the index at 0, but the
  // row's commands must still turn on.
  const [isEngaged, setEngaged] = useState(false)
  const navigate = useNavigate()
  const retry = useRetryJob()
  const promote = usePromoteJob()

  // Clamp focus on jobs-length change (filter, page transition).
  useEffect(() => {
    if (jobs.length === 0 || !engaged.current) return
    if (focusedIndex >= jobs.length) {
      setFocusedIndex(jobs.length - 1)
      onFocusChange?.(jobs.length - 1)
    }
  }, [jobs.length, focusedIndex, onFocusChange])

  const move = useCallback(
    (delta: number) => {
      if (jobs.length === 0) return
      setFocusedIndex((i) => {
        // The first step lands on the current row instead of leaving it
        // (a page jump is a jump all the same).
        const from = engaged.current || Math.abs(delta) > 1 ? i + delta : i
        engaged.current = true
        setEngaged(true)
        const next = Math.max(0, Math.min(jobs.length - 1, from))
        onFocusChange?.(next)
        return next
      })
    },
    [jobs.length, onFocusChange],
  )

  const drillIn = useCallback(() => {
    const job = engaged.current ? jobs[focusedIndex] : undefined
    if (!job) return
    if (onOpen) {
      onOpen(job)
      return
    }
    void navigate({
      to: "/connections/$connectionId/queues/$queueId/$jobId",
      params: {
        connectionId,
        queueId: job.jobRef.queueId,
        jobId: job.jobRef.id,
      },
    })
  }, [connectionId, focusedIndex, jobs, navigate, onOpen])

  const fireAction = useCallback(
    (kind: "retry" | "remove" | "promote") => {
      const job = engaged.current ? jobs[focusedIndex] : undefined
      if (!job) return
      const selection = useJobSelectionStore.getState()
      if (selection.selection.size > 0) return
      if (kind === "remove") {
        selection.add({ jobRef: job.jobRef, state: job.state })
        selection.requestAction("remove")
        return
      }
      const input = {
        connectionId,
        queueId: job.jobRef.queueId,
        jobId: job.jobRef.id,
      }
      if (kind === "retry") retry.mutate(input)
      else promote.mutate(input)
    },
    [connectionId, focusedIndex, jobs, promote, retry],
  )

  // Job › Retry (⇧⌘R) / Promote (⌥⌘P) / Remove… (⌘⌫) and
  // Edit › Copy Job ID (⇧⌘C) act on the focused row once the keyboard has
  // been used here. A selection outranks it (BulkActionBar, priority 2).
  // Removing the row goes through the bulk bar's confirmation.
  const focused = isEngaged ? jobs[focusedIndex] : undefined
  const copy = useCopyToClipboard()
  const { can } = useCapabilities(connectionId)
  useCommandHandler("job.retry", () => fireAction("retry"), {
    enabled: can("jobActions.single") && focused?.state === "failed",
    priority: 1,
  })
  useCommandHandler("job.promote", () => fireAction("promote"), {
    enabled: can("jobActions.single") && focused?.state === "delayed",
    priority: 1,
  })
  useCommandHandler("job.remove", () => fireAction("remove"), {
    enabled: can("jobActions.bulk") && Boolean(focused),
    priority: 1,
  })
  useCommandHandler("job.copyId", () => focused && copy(focused.jobRef.id, "Job ID"), {
    enabled: Boolean(focused),
    priority: 1,
  })
  useCommandHandler(
    "job.openInNewWindow",
    () =>
      focused &&
      openRouteOrSay(jobRoute(connectionId, focused.jobRef.queueId, focused.jobRef.id), {
        tab: false,
      }),
    { enabled: Boolean(focused), priority: 1 },
  )

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTypingTarget(e.target)) return
      // Single-key motion: j/k/arrows/PageUp/PageDown.
      if (!e.metaKey && !e.ctrlKey && !e.altKey) {
        if (e.key === "j" || e.key === "ArrowDown") {
          e.preventDefault()
          move(1)
          return
        }
        if (e.key === "k" || e.key === "ArrowUp") {
          e.preventDefault()
          move(-1)
          return
        }
        if (e.key === "PageDown") {
          e.preventDefault()
          move(10)
          return
        }
        if (e.key === "PageUp") {
          e.preventDefault()
          move(-10)
          return
        }
        if (e.key === "Enter") {
          e.preventDefault()
          drillIn()
          return
        }
      }
      // The action chords are registry commands: see above.
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [drillIn, move])

  return { focusedIndex, setFocusedIndex }
}
