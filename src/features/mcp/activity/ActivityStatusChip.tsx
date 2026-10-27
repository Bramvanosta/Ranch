import {
  BanIcon,
  CheckIcon,
  CircleAlertIcon,
  type LucideIcon,
  PowerOffIcon,
  TimerOffIcon,
  XIcon,
} from "lucide-react"

import type { ActivityStatus } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

/**
 * Icon + label + color: never color alone. Icons, not characters:
 * macOS draws ⛔ and ⏱ as colour emoji, next to a plain ✓.
 */
const STATUS: Record<ActivityStatus, { label: string; icon: LucideIcon; className: string }> = {
  success: {
    label: "Success",
    icon: CheckIcon,
    className: "text-(--state-completed-text)",
  },
  error: { label: "Error", icon: CircleAlertIcon, className: "text-(--state-failed-text)" },
  blocked_readonly: {
    label: "Blocked (read-only)",
    icon: BanIcon,
    className: "text-(--state-delayed-text)",
  },
  denied: { label: "Denied by you", icon: XIcon, className: "text-(--state-failed-text)" },
  timeout: {
    label: "Timed out",
    icon: TimerOffIcon,
    className: "text-(--state-delayed-text)",
  },
  host_unavailable: {
    label: "Ranch was closed",
    icon: PowerOffIcon,
    className: "text-(--text2)",
  },
}

export const ACTIVITY_STATUS_OPTIONS = (Object.keys(STATUS) as ActivityStatus[]).map((value) => ({
  value,
  label: STATUS[value].label,
}))

export function ActivityStatusChip({ status }: { status: ActivityStatus }) {
  const s = STATUS[status]
  return (
    <span
      className={cn("inline-flex items-center gap-1 text-xs font-medium", s.className)}
      data-testid={`activity-status-${status}`}
    >
      <s.icon className="size-3.5 shrink-0" aria-hidden="true" />
      {s.label}
    </span>
  )
}
