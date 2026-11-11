import { useEffect, useRef, useState } from "react"

import { useDecideApproval } from "@/data/mcp/mutations"
import { useMcpSafetySettings, usePendingApprovals } from "@/data/mcp/queries"
import type { PendingApproval } from "@/lib/tauri/bindings"
import { useApprovalUiStore } from "@/stores/useApprovalUiStore"

import { raiseAttention, releaseAttention } from "./approvalAttention"
import { describeRequest } from "./describeApproval"
import { McpApprovalModal } from "./McpApprovalModal"

/** Is a modal other than ours open? Modals never stack (UX modal rules). */
function otherModalOpen(): boolean {
  return Array.from(document.querySelectorAll('[role="dialog"], [role="alertdialog"]')).some(
    (el) =>
      !(el as HTMLElement).closest('[data-testid="mcp-approval-modal"]') &&
      (el as HTMLElement).dataset.testid !== "mcp-approval-modal",
  )
}

/**
 * Mounted once at the root. Shows pending agent requests one at a time,
 * oldest first, each with its own decision; raises the window when a new
 * one arrives; tells the human when a request expired under their eyes.
 *
 * A request the human decided never shows again, even if a list read before
 * the decision lands after it. A request they closed without deciding is set
 * aside: it keeps waiting (Agents › Pending Approvals brings it back) and the
 * next one shows.
 */
export function McpApprovalGate() {
  const pending = usePendingApprovals()
  const decide = useDecideApproval()
  const settings = useMcpSafetySettings().data

  const deferred = useApprovalUiStore((s) => s.deferred)
  const setDeferred = useApprovalUiStore((s) => s.setDeferred)
  const showRequest = useApprovalUiStore((s) => s.showRequest)

  // The request on screen. Kept as a snapshot so it can still be rendered
  // (as "expired") after it left the queue.
  const [shown, setShown] = useState<PendingApproval | null>(null)
  const [expired, setExpired] = useState(false)
  // Never pruned: an id leaves the list before a stale read can bring it
  // back, and one id per decision is nothing to keep.
  const decidedIds = useRef(new Set<string>())
  const announcedIds = useRef(new Set<string>())
  const [setAside, setSetAside] = useState<ReadonlySet<string>>(() => new Set())

  // Agents › Pending Approvals: everything set aside shows again.
  const lastShowRequest = useRef(showRequest)
  useEffect(() => {
    if (showRequest === lastShowRequest.current) return
    lastShowRequest.current = showRequest
    setSetAside(new Set())
  }, [showRequest])

  const next = pending.find((p) => !decidedIds.current.has(p.id) && !setAside.has(p.id)) ?? null

  // Pick what to show.
  useEffect(() => {
    if (shown) {
      const stillPending = pending.some((p) => p.id === shown.id)
      if (!stillPending && !decidedIds.current.has(shown.id) && !expired) {
        setExpired(true) // vanished while the human was looking at it
      }
      return
    }
    if (!next) {
      if (deferred) setDeferred(false)
      return
    }
    if (otherModalOpen() && showRequest === 0) {
      if (!deferred) setDeferred(true)
      return
    }
    setDeferred(false)
    setExpired(false)
    setShown(next)
  }, [pending, next, shown, expired, deferred, setDeferred, showRequest])

  // While held back by another modal, look again regularly.
  useEffect(() => {
    if (!deferred) return
    const timer = window.setInterval(() => {
      if (!otherModalOpen()) setDeferred(false)
    }, 400)
    return () => window.clearInterval(timer)
  }, [deferred, setDeferred])

  // Attention: once per request; released when the queue is empty.
  useEffect(() => {
    const fresh = pending.filter((p) => !announcedIds.current.has(p.id))
    if (fresh.length > 0) {
      for (const p of fresh) announcedIds.current.add(p.id)
      void raiseAttention({
        alwaysOnTop: settings?.approvalAlwaysOnTop ?? true,
        notifySound: settings?.approvalNotifySound ?? true,
        title: "Ranch: an agent needs your approval",
        body: `${describeRequest(fresh[0])} on ${fresh[0].groupName} / ${fresh[0].environmentName}`,
      })
    }
    if (pending.length === 0 && announcedIds.current.size > 0) {
      announcedIds.current.clear()
      void releaseAttention()
    }
  }, [pending, settings?.approvalAlwaysOnTop, settings?.approvalNotifySound])

  if (!shown) return null

  const position = Math.max(1, pending.findIndex((p) => p.id === shown.id) + 1)

  function close() {
    setShown(null)
    setExpired(false)
  }

  return (
    <McpApprovalModal
      approval={shown}
      position={position}
      total={Math.max(pending.length, 1)}
      expired={expired}
      deciding={decide.isPending}
      onAcknowledgeExpired={close}
      onSetAside={() => {
        const id = shown.id
        setSetAside((old) => new Set(old).add(id))
        close()
      }}
      onDecide={(decision, reason) => {
        const id = shown.id
        decidedIds.current.add(id)
        decide.mutate(
          { approvalId: id, decision, reason },
          {
            // `alreadyDecided` (it expired a moment ago) is not an error for
            // the human: just move on to the next request.
            onSuccess: close,
            // The hook says the decision was not recorded; the request shows again.
            onError: () => {
              decidedIds.current.delete(id)
            },
          },
        )
      }}
    />
  )
}
