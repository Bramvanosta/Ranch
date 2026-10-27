import { CircleXIcon, CopyIcon, RotateCwIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import type { Job } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { summarizeError } from "./jobPreview"

/**
 * Why the job failed — always on screen, whatever tab is open: it is the
 * first thing anyone opening a failed job wants to read. It also says where
 * the job stands with its retries:
 *
 *   - failed for good (red): "Failed after 3 of 3 attempts";
 *   - failed once, BullMQ will try again (amber): BullMQ keeps the last
 *     `failedReason` on a job that is waiting for its next attempt — and on
 *     one that completed afterwards.
 */
interface JobFailureBannerProps {
  job: Job
  maxAttempts: number | null
  /** Present when there is a stack trace that is not on screen. */
  onShowStack?: () => void
}

export function JobFailureBanner({ job, maxAttempts, onShowStack }: JobFailureBannerProps) {
  const copy = useCopyToClipboard()
  if (!job.failedReason) return null

  // Red when it failed for good; amber (the app's warning colour) when the
  // error belongs to an attempt that is not the last word.
  const final = job.state === "failed"
  const text = final ? "text-(--state-failed-text)" : "text-(--state-delayed-text)"

  return (
    <section
      role={final ? "alert" : "status"}
      data-testid="job-failure-banner"
      data-final={final ? "true" : "false"}
      className={cn(
        "flex items-start gap-3 rounded-xl border px-4 py-3",
        final
          ? "border-[color-mix(in_oklch,var(--state-failed)_40%,transparent)] bg-[color-mix(in_oklch,var(--state-failed)_8%,transparent)]"
          : "border-(--state-delayed)/40 bg-(--state-delayed)/10",
      )}
    >
      <span className={cn("mt-0.5 shrink-0", text)}>
        {final ? (
          <CircleXIcon className="size-4" aria-hidden="true" />
        ) : (
          <RotateCwIcon className="size-4" aria-hidden="true" />
        )}
      </span>
      <div className="min-w-0 flex-1">
        {/* The message first — for a multi-line error (Prisma: a path and a
            code frame, the message last) that is `summarizeError`, with the
            whole text one click away. */}
        <p
          className={cn("font-mono text-sm font-medium break-words", text)}
          data-testid="job-failure-reason"
        >
          {summarizeError(job.failedReason)}
        </p>
        <p className="mt-1 text-xs text-(--text2)" data-testid="job-failure-status">
          {failureStatus(job, maxAttempts)}
        </p>
        {job.failedReason.trim().includes("\n") ? (
          <details className="mt-2" data-testid="job-failure-full">
            <summary className="cursor-pointer text-xs text-(--text2) select-none hover:text-(--text)">
              Full message
            </summary>
            <pre className="mt-2 max-h-72 overflow-auto rounded-md border border-(--line)/40 bg-(--bg)/60 p-3 font-mono text-xs leading-relaxed break-words whitespace-pre-wrap">
              {job.failedReason.trim()}
            </pre>
          </details>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {onShowStack ? (
          <Button type="button" size="sm" variant="ghost" onClick={onShowStack}>
            Stack trace
          </Button>
        ) : null}
        <Button
          type="button"
          size="sm"
          variant="ghost"
          onClick={() =>
            copy(
              [job.failedReason, ...job.stacktrace].filter(Boolean).join("\n\n"),
              "error and stack trace",
            )
          }
          aria-label="Copy the error and its stack trace"
          title="Copy the error and its stack trace"
          data-testid="job-drill-down-copy-stack"
          className="gap-1.5"
        >
          <CopyIcon className="size-3.5" aria-hidden="true" />
          Copy
        </Button>
      </div>
    </section>
  )
}

export function failureStatus(job: Job, maxAttempts: number | null): string {
  const made = Math.max(job.attemptsMade, 1)
  if (job.state === "failed") {
    if (maxAttempts !== null && maxAttempts > 1) {
      return made >= maxAttempts
        ? `Failed after ${made} of ${maxAttempts} attempts — no retry left.`
        : `Failed on attempt ${made} of ${maxAttempts} — BullMQ stopped retrying it.`
    }
    return made > 1
      ? `Failed after ${made} attempts.`
      : "Failed on its only attempt — this job has no retries configured."
  }
  if (job.state === "completed") {
    // BullMQ does not clear `failedReason` when a later attempt succeeds.
    return "An earlier attempt failed with this error — the job then completed."
  }
  const of = maxAttempts !== null ? ` of ${maxAttempts}` : ""
  return job.state === "active"
    ? `Attempt ${made}${of} failed with this error — it is being retried now.`
    : `Attempt ${made}${of} failed with this error — it will be retried.`
}
