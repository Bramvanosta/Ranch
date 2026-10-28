/**
 * A JSON value as YAML, for the job inspector's Raw tab.
 *
 * Block style, two-space indents. A string that YAML would read as
 * something else (a number, `true`, `null`, a leading `-` or `:`…) is
 * double-quoted with JSON's escaping, which is valid YAML; a multi-line
 * string is a `|` literal block. Enough for job payloads, no dependency.
 */
export function toYaml(value: unknown): string {
  return `${emit(value, 0).replace(/^[\n ]/, "")}\n`
}

const INDENT = "  "

function emit(value: unknown, depth: number): string {
  if (Array.isArray(value)) {
    if (value.length === 0) return " []"
    const pad = INDENT.repeat(depth)
    return value.map((item) => `\n${pad}-${inline(item, depth + 1)}`).join("")
  }
  if (isObject(value)) {
    const entries = Object.entries(value)
    if (entries.length === 0) return " {}"
    const pad = INDENT.repeat(depth)
    return entries.map(([k, v]) => `\n${pad}${key(k)}:${emit(v, depth + 1)}`).join("")
  }
  return ` ${scalar(value, depth)}`
}

/** An array item: a mapping starts on the dash's line. */
function inline(item: unknown, depth: number): string {
  if (isObject(item) && Object.keys(item).length > 0) {
    const body = emit(item, depth)
    // "\n<pad>key: v\n<pad>key2: v" → " key: v\n<pad>key2: v"
    return ` ${body.slice(1 + INDENT.repeat(depth).length)}`
  }
  return emit(item, depth)
}

function scalar(value: unknown, depth: number): string {
  if (value === null || value === undefined) return "null"
  if (typeof value === "boolean" || typeof value === "number") return String(value)
  const s = String(value)
  if (s.includes("\n")) {
    const pad = INDENT.repeat(depth)
    const keep = s.endsWith("\n") ? "" : "-"
    const lines = s.replace(/\n$/, "").split("\n")
    return `|${keep}${lines.map((l) => (l === "" ? "\n" : `\n${pad}${l}`)).join("")}`
  }
  return needsQuotes(s) ? JSON.stringify(s) : s
}

function key(k: string): string {
  return needsQuotes(k) ? JSON.stringify(k) : k
}

function needsQuotes(s: string): boolean {
  return (
    s === "" ||
    s !== s.trim() ||
    /^[-?:,[\]{}#&*!|>'"%@`]/.test(s) ||
    /: |\s#|:$/.test(s) ||
    /^(true|false|yes|no|on|off|null|~)$/i.test(s) ||
    /^[-+]?(\d[\d_]*(\.\d*)?|\.\d+)([eE][-+]?\d+)?$/.test(s) ||
    /^0x[0-9a-f]+$/i.test(s) ||
    [...s].some((c) => c.charCodeAt(0) < 0x20)
  )
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v)
}
