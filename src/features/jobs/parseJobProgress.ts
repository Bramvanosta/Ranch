/**
 * What a worker reported with `job.updateProgress()`.
 *
 * BullMQ accepts a number or an object; `Job.progress` carries the raw JSON
 * text (`null` when nothing was reported). A number is read as a percentage
 * (the BullMQ convention) and clamped to 0–100.
 *
 * An object that carries a percentage (`percent`,
 * `percentage`, `progress`, `pct`) shows as a bar too, and its words
 * (`message`, `msg`, `step`, `stage`, `status`, `note`) as the progress
 * note; anything else is shown as compact JSON, never interpreted. Nothing
 * is cut: the row wraps.
 */
export interface JobProgressValue {
  /** 0–100 when the worker reported a percentage, else `null`. */
  percent: number | null
  /** The value as text: "42%", or the JSON / string reported. */
  label: string
  /** What the worker said it is doing, when it said. */
  note: string | null
}

const PERCENT_KEYS = ["percent", "percentage", "progress", "pct"]
const NOTE_KEYS = ["message", "msg", "step", "stage", "status", "note"]

export function parseJobProgress(raw: string | null | undefined): JobProgressValue | null {
  if (raw === null || raw === undefined) return null
  const text = raw.trim()
  if (text === "") return null
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch {
    return { percent: null, label: text, note: null }
  }
  if (typeof value === "number" && Number.isFinite(value)) {
    const percent = clamp(value)
    return { percent, label: `${Math.round(percent)}%`, note: null }
  }
  if (value === null) return null
  if (typeof value === "string") return { percent: null, label: value, note: null }
  if (typeof value === "object" && !Array.isArray(value)) {
    const record = value as Record<string, unknown>
    const key = PERCENT_KEYS.find(
      (k) => typeof record[k] === "number" && Number.isFinite(record[k] as number),
    )
    const noteKey = NOTE_KEYS.find((k) => typeof record[k] === "string" && record[k] !== "")
    const note = noteKey ? String(record[noteKey]) : null
    if (key) {
      const percent = clamp(record[key] as number)
      return { percent, label: `${Math.round(percent)}%`, note }
    }
    return { percent: null, label: JSON.stringify(value), note }
  }
  return { percent: null, label: JSON.stringify(value), note: null }
}

function clamp(n: number): number {
  return Math.min(100, Math.max(0, n))
}
