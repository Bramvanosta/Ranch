/**
 * One-line previews for the job table. A queue often holds hundreds of jobs
 * with the same name (`send-email`, `enrich-film`): what tells them apart is
 * their payload — so the row shows a glimpse of it.
 */

const MAX_VALUE = 36

/**
 * `{"filmId":603,"source":"tmdb"}` → `filmId: 603 · source: tmdb`. Empty → `""`.
 *
 * `alreadyShown` is text the row displays anyway (the job id): a value it
 * contains is left out — `enrich-film-<uuid>` next to `filmId: <uuid>` spent
 * the whole cell saying the same thing twice.
 */
export function previewJobData(
  data: string | null | undefined,
  maxLength = 160,
  alreadyShown = "",
): string {
  if (!data) return ""
  let parsed: unknown
  try {
    parsed = JSON.parse(data)
  } catch {
    return clip(oneLine(data), maxLength)
  }
  if (parsed === null || parsed === undefined) return ""
  if (Array.isArray(parsed)) {
    return parsed.length === 0 ? "" : clip(parsed.map(previewValue).join(", "), maxLength)
  }
  if (typeof parsed !== "object") return clip(previewValue(parsed), maxLength)

  const parts: string[] = []
  let length = 0
  for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
    if (isRedundant(value, alreadyShown)) continue
    const part = `${key}: ${previewValue(value)}`
    parts.push(part)
    length += part.length + 3
    if (length > maxLength) break
  }
  return clip(parts.join(" · "), maxLength)
}

function isRedundant(value: unknown, alreadyShown: string): boolean {
  const text = typeof value === "number" ? String(value) : value
  return typeof text === "string" && text.length >= 6 && alreadyShown.includes(text)
}

function previewValue(value: unknown): string {
  if (value === null) return "null"
  if (typeof value === "string") {
    const text = oneLine(value)
    // An identifier (uuid, hash, token): both ends tell it apart, the middle does not.
    if (text.length > 20 && !text.includes(" ")) return `${text.slice(0, 8)}…${text.slice(-4)}`
    return clip(text, MAX_VALUE)
  }
  if (typeof value === "number" || typeof value === "boolean") return String(value)
  if (Array.isArray(value)) return `[${value.length}]`
  if (typeof value === "object") return "{…}"
  return String(value)
}

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
 * code frame — and the actual message ("Unique constraint failed on…") on
 * the **last** line. Collapsing that to one line buried the message under
 * the path. So: noise lines (paths, code frames, stack frames) are dropped;
 * a Prisma error reads `<message> — <call>`; any other multi-line error
 * reads `<first line> — <last line>` when the two differ.
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
  if (prismaCall && last !== first) return `${last} — ${prismaCall}`
  return last !== first ? `${first} — ${last}` : first
}

function isNoiseLine(line: string): boolean {
  return (
    /^(→|>)?\s*\d+(\s|$)/.test(line) || // numbered code frame
    /^at\s/.test(line) || // stack frame
    /^([/~]|[A-Za-z]:\\)\S*$/.test(line) // a bare file path
  )
}

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text
}

/** `attemptsMade / opts.attempts` — `3/3`; just `2` when the job sets no limit. */
export function attemptsLabel(attemptsMade: number, opts: string | null | undefined): string {
  let max: unknown
  try {
    max = opts ? (JSON.parse(opts) as { attempts?: unknown }).attempts : undefined
  } catch {
    max = undefined
  }
  return typeof max === "number" && max > 0 ? `${attemptsMade}/${max}` : String(attemptsMade)
}
