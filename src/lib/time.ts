/**
 * Moments as every screen shows them, in the user's locale: one format for a
 * date and time, one for a time of day. Callers keep these strings on one
 * line (`whitespace-nowrap`): a 12-hour clock must not wrap its "PM" under
 * the time in a narrow column.
 */

type When = number | string | Date

function toDate(when: When): Date | null {
  const d = when instanceof Date ? when : new Date(when)
  return Number.isNaN(d.getTime()) ? null : d
}

/** "Oct 28, 09:00 AM", or "Oct 28, 09:00:12 AM" with `seconds`. */
export function formatMoment(when: When, { seconds = false }: { seconds?: boolean } = {}): string {
  const d = toDate(when)
  if (!d) return String(when)
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    ...(seconds ? { second: "2-digit" } : {}),
  })
}

/** "09:00:12 AM", a time of day, for lists that only span today. */
export function formatClock(when: When): string {
  const d = toDate(when)
  if (!d) return String(when)
  return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" })
}
