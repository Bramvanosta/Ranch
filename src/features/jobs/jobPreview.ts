/**
 * A failure reason on one line. ORMs and HTTP clients love multi-line
 * messages that open with a blank line: collapsed, the row shows the message
 * instead of its first, emptiest, line.
 */
export function oneLine(text: string): string {
  return text.replace(/\s+/g, " ").trim()
}

/**
 * A failure reason as one readable line.
 *
 * Most errors say it all on their first line. Prisma does the opposite:
 * `Invalid \`tx.movie.createMany()\` invocation in`, a file path, a numbered
 * code frame, and the actual message ("Unique constraint failed on…") on
 * the **last** line. Collapsing that to one line buried the message under
 * the path. So: noise lines (paths, code frames, stack frames) are dropped;
 * a Prisma error reads `<message> (in <call>)`; any other multi-line error
 * reads `<first line> / <last line>` when the two differ.
 */
export function summarizeError(reason: string): string {
  const lines = reason
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean)
  if (lines.length <= 1) return oneLine(reason)

  const meaningful = lines.filter((l) => !isNoiseLine(l))
  const first = meaningful[0] ?? lines[0] ?? ""
  const last = meaningful[meaningful.length - 1] ?? first

  const prismaCall = /^Invalid `(.+?)` invocation/.exec(lines[0] ?? "")?.[1]
  if (prismaCall && last !== first) return `${last} (in ${prismaCall})`
  return last !== first ? `${first} / ${last}` : first
}

function isNoiseLine(line: string): boolean {
  return (
    /^(→|>)?\s*\d+(\s|$)/.test(line) || // numbered code frame
    /^at\s/.test(line) || // stack frame
    /^([/~]|[A-Za-z]:\\)\S*$/.test(line) // a bare file path
  )
}

/** `attemptsMade / opts.attempts`: `3/3`; just `2` when the job sets no limit. */
export function attemptsLabel(attemptsMade: number, opts: string | null | undefined): string {
  let max: unknown
  try {
    max = opts ? (JSON.parse(opts) as { attempts?: unknown }).attempts : undefined
  } catch {
    max = undefined
  }
  return typeof max === "number" && max > 0 ? `${attemptsMade}/${max}` : String(attemptsMade)
}
