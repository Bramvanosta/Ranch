/**
 * Pure URL parser for the `ranch://` scheme.
 *
 * Grammar (paths are nested; trailing slashes ignored):
 *   - `ranch://<connection-id>`                           → connection target
 *   - `ranch://<connection-id>/queues/<queue>`            → queue target
 *   - `ranch://<connection-id>/queues/<queue>/jobs/<job-id>` → job target
 *
 * `<queue>` is the queue's id, or its name: the queue route finds either.
 *
 * Returns `null` for anything that doesn't match the grammar (other
 * schemes, missing host, garbage segments). Permissive on trailing
 * slashes and on URL-encoded characters within the segments.
 */
export interface RanchTarget {
  connectionId: string
  /** The queue's id or its name, as the link gives it. */
  queueName?: string
  jobId?: string
}

export function parseRanchUrl(url: string): RanchTarget | null {
  if (!url || typeof url !== "string") return null
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    return null
  }
  if (parsed.protocol !== "ranch:") return null

  // The `URL` parser puts the connection id in `hostname` for a URL
  // shaped like `ranch://abc/...`. We accept both with and without the
  // leading `//` (canonical form is `ranch://<id>/...`).
  // A link is anyone's text (a web page can open `ranch://…`): a malformed
  // escape is no link, not an exception.
  const connectionId = tryDecode(parsed.hostname || "")
  if (!connectionId) return null

  // Strip leading + trailing slashes, then split.
  const path = parsed.pathname.replace(/^\/+|\/+$/g, "")
  if (path === "") {
    return { connectionId }
  }

  const decoded = path.split("/").map(tryDecode)
  if (decoded.some((seg) => seg === null)) return null
  const segs = decoded as string[]
  // Valid shapes:
  //   queues/<name>
  //   queues/<name>/jobs/<id>
  if (segs[0] !== "queues" || segs.length < 2 || !segs[1]) {
    return null
  }
  const queueName = segs[1]
  if (segs.length === 2) {
    return { connectionId, queueName }
  }
  if (segs.length === 4 && segs[2] === "jobs" && segs[3]) {
    return { connectionId, queueName, jobId: segs[3] }
  }
  // Anything else (e.g. extra trailing segments, wrong literal) → null.
  return null
}

// ─────────────────────────────────────────────────────────────────────────────
// Named form: ranch://{group}/{environment}[/{queue}[/{job}]]
// ─────────────────────────────────────────────────────────────────────────────

/**
 * The runbook-friendly form. Names instead of ids, so a link written in
 * an alert template survives re-creating a connection:
 *
 *   ranch://lumen/production
 *   ranch://lumen/production/email-digest
 *   ranch://lumen/production/email-digest/42
 *   ranch://lumen/production/email-digest?connection=queues-redis
 *
 * An environment may hold several connections; `?connection=<name>` picks one,
 * otherwise the first in sidebar order is used.
 *
 * Tried only after `parseRanchUrl` (the id form) returned `null`, so
 * `ranch://<id>/queues/...` keeps its meaning.
 */
export interface NamedRanchTarget {
  group: string
  environment: string
  connectionName?: string
  queueName?: string
  jobId?: string
}

export function parseNamedRanchUrl(url: string): NamedRanchTarget | null {
  if (!url || typeof url !== "string") return null
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    return null
  }
  if (parsed.protocol !== "ranch:") return null

  const group = safeDecode(parsed.hostname || "")
  const segs = parsed.pathname
    .replace(/^\/+|\/+$/g, "")
    .split("/")
    .filter(Boolean)
    .map(safeDecode)
  if (!group || segs.length < 1 || segs.length > 3) return null

  const [environment, queueName, jobId] = segs
  const connectionName = parsed.searchParams.get("connection") ?? undefined
  return {
    group,
    environment,
    ...(connectionName ? { connectionName } : {}),
    ...(queueName ? { queueName } : {}),
    ...(jobId ? { jobId } : {}),
  }
}

function tryDecode(s: string): string | null {
  try {
    return decodeURIComponent(s)
  } catch {
    return null
  }
}

function safeDecode(s: string): string {
  try {
    return decodeURIComponent(s)
  } catch {
    return s
  }
}

interface ResolvableConnection {
  id: string
  name: string
  groupName: string
  envLabel: string
}

export type NamedResolution =
  | { ok: true; target: RanchTarget; ambiguous: boolean }
  | { ok: false; reason: string }

/**
 * Resolve a named target against the saved connections, case-insensitively,
 * because URL hosts get lower-cased on their way through the OS.
 */
export function resolveNamedTarget(
  named: NamedRanchTarget,
  connections: ResolvableConnection[],
): NamedResolution {
  const eq = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase()

  const inGroup = connections.filter((c) => eq(c.groupName, named.group))
  if (inGroup.length === 0) {
    return { ok: false, reason: `We couldn't find a group named "${named.group}".` }
  }
  const inEnv = inGroup.filter((c) => eq(c.envLabel, named.environment))
  if (inEnv.length === 0) {
    return {
      ok: false,
      reason: `Group "${named.group}" has no environment named "${named.environment}".`,
    }
  }

  let candidates = inEnv
  if (named.connectionName) {
    const wanted = named.connectionName
    candidates = inEnv.filter((c) => eq(c.name, wanted))
    if (candidates.length === 0) {
      return {
        ok: false,
        reason: `No connection named "${wanted}" in ${named.group} / ${named.environment}.`,
      }
    }
  }

  return {
    ok: true,
    ambiguous: candidates.length > 1,
    target: {
      connectionId: candidates[0].id,
      ...(named.queueName ? { queueName: named.queueName } : {}),
      ...(named.jobId ? { jobId: named.jobId } : {}),
    },
  }
}
