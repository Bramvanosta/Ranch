import { ShieldAlertIcon, TriangleAlertIcon } from "lucide-react"
import { type ReactNode, useId, useRef, useState } from "react"

import { Button } from "@/components/ui/button"
import { Sheet } from "@/components/ui/sheet"

export interface CountLine {
  label: string
  count?: number
}

/**
 * A confirmation that says exactly what goes and what stays:
 *
 *   - an icon and a question for a title;
 *   - what is removed and what is kept, with counts;
 *   - on a protected (read-only) connection, a note and a field where the
 *     user types `protectedName` before the action can run;
 *   - Cancel focused by default when the action destroys something, and a
 *     button that says how much it does ("Drain 400 jobs").
 */
export function ConfirmSheet({
  open,
  onOpenChange,
  title,
  description,
  removed = [],
  kept = [],
  danger = false,
  protectedName,
  confirmLabel,
  onConfirm,
  busy = false,
  disabled = false,
  children,
  testId,
  confirmTestId,
  cancelTestId,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: ReactNode
  description?: ReactNode
  removed?: CountLine[]
  kept?: CountLine[]
  danger?: boolean
  /** Set on a protected connection: the name to type before confirming. */
  protectedName?: string
  confirmLabel: string
  onConfirm: () => void
  busy?: boolean
  /** Not ready yet (counting, nothing to do). */
  disabled?: boolean
  /** Options, a preview, progress: between the counts and the buttons. */
  children?: ReactNode
  testId?: string
  confirmTestId?: string
  cancelTestId?: string
}) {
  const cancelRef = useRef<HTMLButtonElement>(null)
  const confirmRef = useRef<HTMLButtonElement>(null)
  const [typed, setTyped] = useState("")
  const fieldId = useId()
  const nameOk = protectedName === undefined || typed === protectedName

  return (
    <Sheet
      open={open}
      onOpenChange={(next) => {
        if (busy && !next) return
        if (!next) setTyped("")
        onOpenChange(next)
      }}
      title={title}
      description={description}
      icon={
        danger ? <TriangleAlertIcon className="text-(--danger)" aria-hidden="true" /> : undefined
      }
      initialFocus={danger || protectedName !== undefined ? cancelRef : confirmRef}
      testId={testId}
      footer={
        <>
          <Button
            ref={cancelRef}
            size="lg"
            variant="default"
            onClick={() => onOpenChange(false)}
            disabled={busy}
            data-testid={cancelTestId}
          >
            Cancel
          </Button>
          <Button
            ref={confirmRef}
            size="lg"
            variant={danger ? "danger" : "primary"}
            onClick={onConfirm}
            disabled={busy || disabled || !nameOk}
            data-testid={confirmTestId}
          >
            {confirmLabel}
          </Button>
        </>
      }
    >
      {removed.length > 0 || kept.length > 0 ? (
        <dl className="grid grid-cols-2 gap-3 text-sm" data-testid="confirm-sheet-counts">
          {removed.length > 0 ? (
            <div className="flex flex-col gap-1 rounded-card border border-(--danger)/30 bg-(--danger)/5 p-3">
              <dt className="text-2xs font-semibold tracking-wide text-(--state-failed-text) uppercase">
                {danger ? "Removed" : "Affected"}
              </dt>
              {removed.map((l) => (
                <dd key={l.label} className="flex justify-between gap-3">
                  <span className="min-w-0 break-words">{l.label}</span>
                  {l.count !== undefined ? (
                    <span className="font-tabular">{l.count.toLocaleString()}</span>
                  ) : null}
                </dd>
              ))}
            </div>
          ) : null}
          {kept.length > 0 ? (
            <div className="flex flex-col gap-1 rounded-card border border-(--line) bg-(--surf) p-3">
              <dt className="text-2xs font-semibold tracking-wide text-(--text2) uppercase">
                Kept
              </dt>
              {kept.map((l) => (
                <dd key={l.label} className="flex justify-between gap-3">
                  <span className="min-w-0 break-words">{l.label}</span>
                  {l.count !== undefined ? (
                    <span className="font-tabular">{l.count.toLocaleString()}</span>
                  ) : null}
                </dd>
              ))}
            </div>
          ) : null}
        </dl>
      ) : null}
      {children}
      {protectedName !== undefined ? (
        <div className="flex flex-col gap-2 rounded-card border border-(--state-delayed)/40 bg-(--state-delayed)/5 p-3 text-sm">
          <p className="flex items-start gap-2">
            <ShieldAlertIcon
              className="mt-0.5 size-4 shrink-0 text-(--state-delayed-text)"
              aria-hidden="true"
            />
            <span>
              This connection is protected. Confirming unlocks it for this action only; it is
              recorded in the audit log.
            </span>
          </p>
          <label htmlFor={fieldId} className="text-xs text-(--text2)">
            Type <span className="font-mono font-semibold text-(--text)">{protectedName}</span> to
            confirm
          </label>
          <input
            id={fieldId}
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            autoComplete="off"
            spellCheck={false}
            data-testid="confirm-sheet-name"
            className="h-control rounded-control border border-(--line) bg-(--field) px-2.5 font-mono text-sm outline-none focus-visible:border-(--accent) focus-visible:ring-3 focus-visible:ring-(--accent)/50"
          />
        </div>
      ) : null}
    </Sheet>
  )
}
