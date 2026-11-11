import { CircleCheckIcon, CircleIcon, CircleXIcon, LoaderCircleIcon } from "lucide-react"

import { ProgressBar } from "@/components/ui/progress-bar"
import type { InstallPhase, InstallProgress } from "@/data/updater/mutations"
import { formatBytes } from "@/domain/format"
import { cn } from "@/lib/utils"

/** The phases of an install, in the order `install_update` goes through them. */
export const INSTALL_STEPS: ReadonlyArray<{ phase: InstallPhase; label: string; doing: string }> = [
  { phase: "downloading", label: "Download", doing: "Downloading" },
  { phase: "verifying", label: "Check the signature", doing: "Checking the signature" },
  { phase: "installing", label: "Install", doing: "Installing" },
  { phase: "restarting", label: "Restart Ranch", doing: "Restarting" },
]

type StepState = "done" | "running" | "failed" | "pending"

/** Whole percents, rounded down: 100 % only once the last byte is in. */
function percentOf(received: number, total: number) {
  return total > 0 ? Math.min(100, Math.floor((received / total) * 100)) : 0
}

/** "4.2 MB of 14 MB", "4.2 MB received", or nothing before the first bytes. */
function downloadDetail(progress: InstallProgress | null, state: StepState) {
  const received = progress?.received ?? null
  if (received === null) return null
  const total = progress?.total ?? null
  if (state === "done") return formatBytes(total ?? received)
  return total === null
    ? `${formatBytes(received)} received`
    : `${formatBytes(received)} of ${formatBytes(total)}`
}

/**
 * Where an install is: the four steps with the one under way marked, the
 * download's bytes and percent when its size is known (a sweeping bar when it
 * is not), and the step that failed, if one did. Before the first report the
 * download is the step under way: the feed is asked again first.
 */
export function InstallSteps({
  progress,
  failed,
}: {
  progress: InstallProgress | null
  failed: boolean
}) {
  const current = Math.max(
    0,
    INSTALL_STEPS.findIndex((s) => s.phase === (progress?.phase ?? "downloading")),
  )
  const doing = INSTALL_STEPS[current]
  return (
    <div className="flex flex-col gap-2">
      <ol
        aria-label="Update progress"
        className="flex flex-col gap-2 rounded-card border border-(--line) bg-(--surf) p-3 text-sm"
        data-testid="updater-progress"
      >
        {INSTALL_STEPS.map((step, index) => {
          const state: StepState =
            index < current ? "done" : index > current ? "pending" : failed ? "failed" : "running"
          const download = step.phase === "downloading"
          const detail = download ? downloadDetail(progress, state) : null
          const total = progress?.total ?? null
          const received = progress?.received ?? 0
          const showBar =
            download && (state === "running" || (state === "failed" && total !== null))
          return (
            <li
              key={step.phase}
              className="flex flex-col gap-1.5"
              data-testid={`updater-step-${step.phase}`}
              data-state={state}
              aria-current={state === "running" ? "step" : undefined}
            >
              <div className="flex min-w-0 items-center gap-2">
                <StepIcon state={state} />
                <span
                  className={cn(
                    "min-w-0 flex-1 break-words",
                    state === "pending" && "text-(--text2)",
                    state === "failed" && "text-(--state-failed-text)",
                    state === "running" && "font-medium",
                  )}
                >
                  {step.label}
                </span>
                {detail ? (
                  <span
                    className="shrink-0 font-tabular text-xs text-(--text2)"
                    data-testid="updater-download-bytes"
                  >
                    {detail}
                  </span>
                ) : null}
              </div>
              {showBar ? (
                <ProgressBar
                  className="pl-6"
                  ariaLabel="Download"
                  tone={state === "failed" ? "warning" : "accent"}
                  value={total === null ? null : percentOf(received, total)}
                  label={total === null ? undefined : `${percentOf(received, total)}%`}
                />
              ) : null}
            </li>
          )
        })}
      </ol>
      {/* Read out once per phase, not once per chunk. */}
      <span className="sr-only" role="status">
        {failed ? `${doing?.label ?? "The update"} failed` : `${doing?.doing ?? "Installing"}`}
      </span>
    </div>
  )
}

function StepIcon({ state }: { state: StepState }) {
  const size = "size-4 shrink-0"
  if (state === "done") {
    return (
      <CircleCheckIcon className={cn(size, "text-(--state-completed-text)")} aria-hidden="true" />
    )
  }
  if (state === "failed") {
    return <CircleXIcon className={cn(size, "text-(--state-failed-text)")} aria-hidden="true" />
  }
  if (state === "running") {
    return (
      <LoaderCircleIcon
        className={cn(size, "animate-spin text-(--accent) motion-reduce:animate-none")}
        aria-hidden="true"
      />
    )
  }
  return <CircleIcon className={cn(size, "text-(--text2)")} aria-hidden="true" />
}
