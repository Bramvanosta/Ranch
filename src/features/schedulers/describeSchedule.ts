import { formatEveryMs } from "@/features/queues/formatEvery"
import type { CronJob, Job } from "@/lib/tauri/bindings"

/**
 * A scheduler's schedule in plain language, beside its
 * pattern: "every 5 minutes", "daily at 09:00", "weekdays at 08:30". The
 * shapes people write; anything else keeps its pattern alone (`null`).
 */
const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"]

const isNum = (s: string) => /^\d+$/.test(s)
const pad = (n: string) => n.padStart(2, "0")

export function describeCron(pattern: string): string | null {
  let parts = pattern.trim().split(/\s+/)
  if (parts.length === 6) {
    // Seconds first: only "on the second 0" reads like a 5-field pattern.
    if (parts[0] !== "0") return null
    parts = parts.slice(1)
  }
  if (parts.length !== 5) return null
  const [m, h, dom, mon, dow] = parts as [string, string, string, string, string]
  if (mon !== "*") return null
  const every = /^\*\/(\d+)$/
  if (m === "*" && h === "*" && dom === "*" && dow === "*") return "every minute"
  const mStep = every.exec(m)
  if (mStep && h === "*" && dom === "*" && dow === "*") {
    return mStep[1] === "1" ? "every minute" : `every ${mStep[1]} minutes`
  }
  if (!isNum(m)) return null
  if (h === "*" && dom === "*" && dow === "*") return `hourly at :${pad(m)}`
  const hStep = every.exec(h)
  if (hStep && dom === "*" && dow === "*") return `every ${hStep[1]} hours at :${pad(m)}`
  if (!isNum(h)) return null
  const at = `${pad(h)}:${pad(m)}`
  if (dom === "*" && dow === "*") return `daily at ${at}`
  if (dom === "*" && (dow === "1-5" || dow === "MON-FRI")) return `weekdays at ${at}`
  if (dom === "*" && isNum(dow) && Number(dow) <= 7)
    return `every ${DAYS[Number(dow) % 7]} at ${at}`
  if (isNum(dom) && dow === "*") return `monthly on day ${dom} at ${at}`
  return null
}

/** What the table's Schedule column says. */
export function describeSchedule(s: Pick<CronJob, "pattern" | "every" | "tz">): {
  main: string
  plain: string | null
} {
  if (s.every !== null) return { main: `every ${formatEveryMs(s.every)}`, plain: null }
  const pattern = s.pattern ?? ""
  const plain = describeCron(pattern)
  return { main: pattern, plain: plain && s.tz ? `${plain} (${s.tz})` : plain }
}

/**
 * The jobs a scheduler made, newest first, among recent jobs of its queue:
 * BullMQ names them `repeat:<scheduler id>:<time>`; older versions keep
 * only the job name, so that matches too.
 */
export function runsOf(scheduler: Pick<CronJob, "key" | "name">, jobs: readonly Job[]): Job[] {
  const prefix = `repeat:${scheduler.key}:`
  const own = jobs.filter((j) => j.jobRef.id.startsWith(prefix))
  const runs = own.length > 0 ? own : jobs.filter((j) => j.name === scheduler.name)
  return [...runs].sort(
    (a, b) =>
      (b.finishedOn ?? b.processedOn ?? b.timestamp) -
      (a.finishedOn ?? a.processedOn ?? a.timestamp),
  )
}
