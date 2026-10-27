/**
 * full-text error popover for the job list table.
 *
 * The trigger is
 * **the reason itself**, truncated to the column (UI review 2026-09-19: it
 * was a "View error" chip — in a list of failures the one thing you want to
 * read was one click away on every row); the popover shows the entire
 * reason plus the stack trace, scrollable so very long stacks stay bounded.
 *
 * Triggering preventDefault on the chip click is critical: the row
 * itself navigates to the drill-down on click, so we must stop the
 * outer handler from firing.
 */
import { useState } from "react"

import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"

import { summarizeError } from "./jobPreview"

interface JobErrorPopoverProps {
  /** The job's `failedReason`. Required — caller filters on null. */
  failedReason: string
  /** Optional stack trace lines. Joined with newlines for display. */
  stacktrace?: string[]
}

export function JobErrorPopover({ failedReason, stacktrace = [] }: JobErrorPopoverProps) {
  const [open, setOpen] = useState(false)

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <button
            type="button"
            data-testid="job-error-popover-trigger"
            // Stop the outer row's click handler from also navigating.
            onClick={(e) => {
              e.stopPropagation()
              e.preventDefault()
            }}
            onMouseDown={(e) => {
              e.stopPropagation()
            }}
            title={failedReason}
            aria-label={`Error: ${failedReason}. Show the stack trace`}
            // The clamp is on the inner span, and the button is cut at two
            // lines' height: WebKit (the app's webview) ignores `line-clamp`
            // on a `<button>` — a ten-line Prisma error spilled over the rows
            // below. Chromium clamps buttons, which is why tests and the web
            // preview never showed it.
            className="block max-h-[31px] w-full min-w-0 overflow-hidden rounded-sm text-left font-mono text-2xs leading-snug text-(--state-failed-text) hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)"
          >
            <span className="line-clamp-2 break-words">{summarizeError(failedReason)}</span>
          </button>
        }
      />
      <PopoverContent
        side="left"
        align="start"
        className="w-[420px]"
        data-testid="job-error-popover-content"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex flex-col gap-2">
          <p className="text-xs font-semibold uppercase tracking-wide text-(--state-failed-text)">
            Failure
          </p>
          <p className="break-words whitespace-pre-wrap text-xs">{failedReason.trim()}</p>
          {stacktrace.length > 0 ? (
            <pre
              className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md border border-(--line)/40 bg-(--surf)/40 p-2 font-mono text-2xs"
              data-testid="job-error-popover-stack"
            >
              {stacktrace.join("\n")}
            </pre>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  )
}
