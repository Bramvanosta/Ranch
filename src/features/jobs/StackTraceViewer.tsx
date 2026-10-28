import { cn } from "@/lib/utils"

/**
 * Render a BullMQ stack trace + optional `failedReason`.
 *
 * BullMQ's `stacktrace` is **one entry per failed attempt**, each the whole
 * `err.stack` of that attempt (oldest first, trimmed to `stackTraceLimit`).
 * So: one block per attempt, the latest on top and open, the earlier ones
 * folded below it. (Entries that are single lines (what some other
 * producers write) read fine too: they are joined into one block.)
 *
 * Inside a block:
 *   - the message line in `--state-failed`;
 *   - the frames of *your* code in the normal foreground, those of
 *     `node_modules` and Node internals dimmed: the eye goes to the frame
 *     that matters;
 *   - `file:line:col` spans are marked, so the location reads at a glance.
 *
 * `showReason={false}` leaves the `failedReason` card out; the job page
 * already shows it above the tabs.
 *
 * Empty `failedReason` AND empty `stacktrace` → an explicit "this job
 * didn't fail" message rather than blank space.
 */
interface StackTraceViewerProps {
  stacktrace: string[]
  failedReason: string | null
  showReason?: boolean
}

/** Matches `at fn (file:line:col)` or `at file:line:col` (V8 / Node style).
 *  Excludes `(`, `)`, and whitespace from the file part so the leading
 *  paren in `at fn (file:line:col)` isn't gobbled into the match. */
const FRAME_REGEX_PARENS = /([^\s()]+):(\d+):(\d+)/

/** Frames nobody debugs: dependencies and the runtime. */
const LIBRARY_FRAME = /node_modules|node:internal|\(node:|<anonymous>/

export function StackTraceViewer({
  stacktrace,
  failedReason,
  showReason = true,
}: StackTraceViewerProps) {
  const hasFailedReason = failedReason !== null && failedReason.length > 0
  const attempts = groupAttempts(stacktrace)

  if (!hasFailedReason && attempts.length === 0) {
    return (
      <div
        className="rounded-md border border-dashed border-(--line) bg-(--surf)/20 p-6 text-center"
        data-testid="stacktrace-empty"
      >
        <p className="text-sm font-medium">This job didn't fail.</p>
        <p className="mt-1 text-xs text-(--text2)">No stack trace recorded.</p>
      </div>
    )
  }

  // Latest failure first; numbered in the order they happened.
  const newestFirst = attempts.map((lines, i) => ({ lines, number: i + 1 })).reverse()

  return (
    <div className="flex flex-col gap-3" data-testid="stacktrace-viewer">
      {hasFailedReason && showReason ? (
        <div
          className="rounded-md border p-3"
          style={{
            borderColor: "color-mix(in oklch, var(--state-failed) 40%, transparent)",
            backgroundColor: "color-mix(in oklch, var(--state-failed) 8%, transparent)",
          }}
          data-testid="stacktrace-reason"
        >
          <p
            className="font-mono text-sm font-medium"
            style={{ color: "var(--state-failed-text)" }}
          >
            {failedReason}
          </p>
        </div>
      ) : null}
      {attempts.length === 0 ? (
        <p
          className="rounded-md border border-dashed border-(--line) bg-(--surf)/20 p-6 text-center text-sm text-(--text2)"
          data-testid="stacktrace-none"
        >
          No stack trace was recorded for this failure.
        </p>
      ) : null}
      {newestFirst.map(({ lines, number }, i) =>
        attempts.length === 1 ? (
          <StackBlock key={number} lines={lines} />
        ) : (
          <details
            key={number}
            open={i === 0}
            className="group rounded-md border border-(--line)/40 bg-(--surf)"
            data-testid="stacktrace-attempt"
          >
            <summary className="flex cursor-pointer items-center gap-2 px-3 py-2 text-xs select-none">
              <span className="font-medium">
                Failure {number} of {attempts.length}
              </span>
              {i === 0 ? (
                <span className="rounded-full bg-(--surf) px-1.5 py-px text-2xs font-medium text-(--text2)">
                  latest
                </span>
              ) : null}
              <span className="min-w-0 flex-1 font-mono wrap-anywhere text-(--text2) group-open:hidden">
                {lines[0]}
              </span>
            </summary>
            <StackBlock lines={lines} nested />
          </details>
        ),
      )}
    </div>
  )
}

function StackBlock({ lines, nested }: { lines: string[]; nested?: boolean }) {
  return (
    <pre
      className={cn(
        "overflow-x-auto p-3 font-mono text-xs leading-relaxed",
        nested ? "border-t border-(--line)/40" : "rounded-md border border-(--line)/40 bg-(--surf)",
      )}
      data-testid="stacktrace-block"
    >
      {lines.map((line, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: lines are content-stable in this view
        <StackLine key={i} line={line} index={i} />
      ))}
    </pre>
  )
}

function StackLine({ line, index }: { line: string; index: number }) {
  const isFirst = index === 0
  const isFrame = /^\s+at\s/.test(line)
  const isLibrary = isFrame && LIBRARY_FRAME.test(line)
  return (
    <div
      className={cn(
        // As wide as the line: the block scrolls sideways, nothing runs over it.
        "w-max min-w-full whitespace-pre",
        isFirst && "font-semibold",
        !isFirst && (isLibrary || !isFrame) && "text-(--text2)",
      )}
      style={isFirst ? { color: "var(--state-failed-text)" } : undefined}
      data-library={isLibrary ? "true" : undefined}
    >
      {renderLineWithFrame(line)}
    </div>
  )
}

/**
 * One list of lines per attempt. An entry with line breaks is a whole
 * `err.stack` (BullMQ); entries without any are single lines of one trace.
 */
export function groupAttempts(stacktrace: string[]): string[][] {
  const entries = stacktrace.filter((s) => s.length > 0)
  if (entries.length === 0) return []
  if (entries.some((s) => s.includes("\n"))) {
    return entries.map((s) => s.split("\n"))
  }
  // No entry spans several lines: either one-line errors, one per attempt…
  // or one trace written line by line. Indented "at" lines settle it.
  return entries.some((s) => /^\s+at\s/.test(s)) ? [entries] : entries.map((s) => [s])
}

/** Highlight any `file:line:col` substring with a click-target affordance.
 *  A click does nothing: Ranch does not open a frame in an editor. */
function renderLineWithFrame(line: string): React.ReactNode {
  const match = FRAME_REGEX_PARENS.exec(line)
  if (!match) return line
  const start = match.index
  const end = start + match[0].length
  const before = line.slice(0, start)
  const frame = line.slice(start, end)
  const after = line.slice(end)
  return (
    <>
      {before}
      <span
        className="underline decoration-dotted underline-offset-2"
        data-testid="stacktrace-frame"
      >
        {frame}
      </span>
      {after}
    </>
  )
}
