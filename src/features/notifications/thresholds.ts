/**
 * Threshold model + edge-trigger detector.
 */

export type ThresholdMetric = "failed" | "active" | "waiting"

export interface NotificationThreshold {
  id: string
  /** Optional: when present, scoped to this connection only. */
  connectionId?: string
  /** Optional: when present, scoped to this queue only. */
  queueName?: string
  /** Counter to watch. */
  metric: ThresholdMetric
  /** Cross-direction: only "gt" supported in v1. */
  op: "gt"
  /** Threshold value (e.g. 10). */
  value: number
}

/**
 * Edge-triggered detector. Given a previous and current counter
 * value, returns `true` if the threshold was JUST crossed
 * (previous below, current above) — i.e. the notification should
 * fire exactly once per crossing.
 */
export function isThresholdCrossing(
  threshold: NotificationThreshold,
  previousValue: number | undefined,
  currentValue: number,
): boolean {
  if (threshold.op !== "gt") return false
  if (currentValue <= threshold.value) return false
  if (previousValue === undefined) {
    // First observation already above threshold — fire on first poll
    // so the user is alerted to the state. Subsequent polls see the
    // previous value (this value), so the edge-trigger logic stays
    // honest from here on.
    return true
  }
  return previousValue <= threshold.value
}

/** Stable key for the last-seen value map: `connectionId::queue::metric`. */
export function thresholdKey(
  connectionId: string,
  queueName: string,
  metric: ThresholdMetric,
): string {
  return `${connectionId}::${queueName}::${metric}`
}

/** Does this threshold apply to (connection, queue)? */
export function thresholdMatches(
  t: NotificationThreshold,
  connectionId: string,
  queueName: string,
): boolean {
  if (t.connectionId && t.connectionId !== connectionId) return false
  if (t.queueName && t.queueName !== queueName) return false
  return true
}
