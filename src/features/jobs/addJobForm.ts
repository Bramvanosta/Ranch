import type { BackoffKind, NewJob } from "@/lib/tauri/bindings"

/**
 * The Add job form, as data (pure, unit-tested). The backend
 * validates again (`NewJob::prepare`): it is the guard; this is the UX:
 * errors next to the field, before anything is sent.
 */
export interface AddJobFormValues {
  name: string
  /** JSON text. */
  data: string
  /** Text inputs: empty = not set. */
  delayMs: string
  attempts: string
  priority: string
  lifo: boolean
  /** BullMQ's backoff between two attempts. */
  backoffKind: "none" | BackoffKind
  backoffDelayMs: string
  /** Empty = the queue's counter. */
  jobId: string
  /** BullMQ keeps finished jobs unless told otherwise. */
  keepCompleted: boolean
  keepFailed: boolean
}

export const EMPTY_ADD_JOB_FORM: AddJobFormValues = {
  name: "",
  data: "{\n  \n}",
  delayMs: "",
  attempts: "",
  priority: "",
  lifo: false,
  backoffKind: "none",
  backoffDelayMs: "",
  jobId: "",
  keepCompleted: true,
  keepFailed: true,
}

/** Mirrors `bullmq_schema::add_job`. */
export const MAX_PRIORITY = 2_097_152
export const MAX_ATTEMPTS = 1_000
export const MAX_DELAY_MS = 10 * 365 * 24 * 60 * 60 * 1000

export type AddJobFormErrors = Partial<Record<keyof AddJobFormValues, string>>

export type AddJobFormResult = { ok: true; job: NewJob } | { ok: false; errors: AddJobFormErrors }

function wholeNumber(text: string): number | null {
  const t = text.trim()
  if (!/^\d+$/.test(t)) return null
  const n = Number(t)
  return Number.isSafeInteger(n) ? n : null
}

/** `null` when the text is valid JSON, else the parser's message. */
export function jsonError(text: string): string | null {
  if (text.trim() === "") return "The payload is required; use {} for an empty one"
  try {
    JSON.parse(text)
    return null
  } catch (e) {
    return e instanceof Error ? e.message : "Not valid JSON"
  }
}

export function validateAddJobForm(values: AddJobFormValues): AddJobFormResult {
  const errors: AddJobFormErrors = {}

  const name = values.name.trim()
  if (name === "") errors.name = "Give the job a name; it is what the worker switches on"
  else if (name.includes(":")) errors.name = "A job name cannot contain ':'"
  else if (name.length > 256) errors.name = "At most 256 characters"

  const dataProblem = jsonError(values.data)
  if (dataProblem) errors.data = dataProblem

  let delayMs = 0
  if (values.delayMs.trim() !== "") {
    const n = wholeNumber(values.delayMs)
    if (n === null) errors.delayMs = "A whole number of milliseconds"
    else if (n > MAX_DELAY_MS) errors.delayMs = "At most 10 years"
    else delayMs = n
  }

  let attempts: number | null = null
  if (values.attempts.trim() !== "") {
    const n = wholeNumber(values.attempts)
    if (n === null || n < 1 || n > MAX_ATTEMPTS) {
      errors.attempts = `Between 1 and ${MAX_ATTEMPTS.toLocaleString()}`
    } else attempts = n
  }

  let priority = 0
  if (values.priority.trim() !== "") {
    const n = wholeNumber(values.priority)
    if (n === null || n > MAX_PRIORITY) {
      errors.priority = `Between 0 and ${MAX_PRIORITY.toLocaleString()} (1 = highest)`
    } else priority = n
  }

  let backoffDelay = 0
  if (values.backoffKind !== "none") {
    const n = wholeNumber(values.backoffDelayMs)
    if (n === null) errors.backoffDelayMs = "A whole number of milliseconds"
    else if (n > MAX_DELAY_MS) errors.backoffDelayMs = "At most 10 years"
    else backoffDelay = n
  }

  // BullMQ's own rules for a custom id (`Job.validateOptions`).
  const jobId = values.jobId.trim()
  if (/^\d+$/.test(jobId)) {
    errors.jobId = "Not a whole number: BullMQ numbers its own jobs"
  } else if (jobId.includes(":")) {
    errors.jobId = "A job ID cannot contain ':'"
  } else if (jobId.length > 256) {
    errors.jobId = "At most 256 characters"
  }

  if (Object.keys(errors).length > 0) return { ok: false, errors }
  return {
    ok: true,
    job: {
      name,
      data: values.data.trim(),
      delayMs,
      attempts,
      priority,
      lifo: values.lifo,
      backoff:
        values.backoffKind === "none" ? null : { kind: values.backoffKind, delayMs: backoffDelay },
      jobId: jobId === "" ? null : jobId,
      removeOnComplete: !values.keepCompleted,
      removeOnFail: !values.keepFailed,
    },
  }
}

/**
 * The same job in code: the `queue.add(...)` a developer would
 * write, from what the form holds (only the options that were set).
 */
export function addJobSnippet(values: AddJobFormValues): string {
  const name = JSON.stringify(values.name.trim() || "job-name")
  let data = "{}"
  try {
    data = JSON.stringify(JSON.parse(values.data), null, 2)
  } catch {
    data = values.data.trim() || "{}"
  }
  const opts: string[] = []
  const n = (text: string) => wholeNumber(text)
  if (n(values.attempts) !== null) opts.push(`attempts: ${n(values.attempts)}`)
  if (values.backoffKind !== "none" && n(values.backoffDelayMs) !== null) {
    opts.push(`backoff: { type: "${values.backoffKind}", delay: ${n(values.backoffDelayMs)} }`)
  }
  if ((n(values.delayMs) ?? 0) > 0) opts.push(`delay: ${n(values.delayMs)}`)
  if ((n(values.priority) ?? 0) > 0) opts.push(`priority: ${n(values.priority)}`)
  if (values.jobId.trim() !== "") opts.push(`jobId: ${JSON.stringify(values.jobId.trim())}`)
  if (!values.keepCompleted) opts.push("removeOnComplete: true")
  if (!values.keepFailed) opts.push("removeOnFail: true")
  if (values.lifo) opts.push("lifo: true")
  const args = [name, data]
  if (opts.length > 0) args.push(`{\n  ${opts.join(",\n  ")},\n}`)
  return `await queue.add(${args.join(", ")})`
}

/** Where BullMQ will put it, shown under the form so the result is no surprise. */
export function describePlacement(values: AddJobFormValues, queuePaused: boolean): string {
  const delay = wholeNumber(values.delayMs) ?? 0
  const priority = wholeNumber(values.priority) ?? 0
  if (delay > 0) {
    return `Goes to delayed: a worker may pick it up in ${delay.toLocaleString()} ms.`
  }
  if (priority > 0) {
    return queuePaused
      ? "Goes to prioritized; the queue is paused, so it waits for a resume."
      : "Goes to prioritized: processed after the waiting jobs, by priority."
  }
  if (queuePaused) return "The queue is paused: the job waits in paused until it is resumed."
  return values.lifo
    ? "Goes to the front of waiting: the next job a worker picks up."
    : "Goes to waiting, behind the jobs already there."
}
