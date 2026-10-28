import {
  DEFAULT_SHARE_MODE,
  DEFAULT_SHARE_PATTERNS,
  isShareMode,
  type ShareMode,
} from "@/domain/aiShare"
import { type AskAgent, isAskAgent } from "@/domain/askAgents"
import { DEFAULT_PAGE_SIZE, isPageSize, type PageSize } from "@/domain/pageSize"
import type { NotificationThreshold } from "@/domain/thresholds"
import type { RecentItem } from "@/stores/useCommandPaletteStore"

import { defineSetting, jsonSetting, type Setting } from "./defineSetting"

/**
 * Every value the front end keeps in `kv_settings`, with the key it is
 * stored under. The keys are part of every existing install: renaming one
 * forgets what users chose.
 */

/** A flag written as `"1"` and cleared as `"0"`; anything but `"1"` is off. */
function flagSetting(key: string): Setting<boolean> {
  return defineSetting(
    key,
    (raw) => raw === "1",
    false,
    (on) => (on ? "1" : "0"),
  )
}

const isArray = (value: unknown): value is unknown[] => Array.isArray(value)

/** Auto-refresh on / off and its base interval (`refreshSettings.ts`). */
export { refreshAuto, refreshIntervalMs } from "./refreshSettings"

export const jobPageSize = defineSetting<PageSize>(
  "jobs.pageSize",
  (raw) => {
    const size = Number(raw)
    return isPageSize(size) ? size : undefined
  },
  DEFAULT_PAGE_SIZE,
)

/** The connection open at the last quit, restored at launch; `""` when none. */
export const lastSelectedConnection = defineSetting<string | null>(
  "selected_connection_id",
  (raw) => raw,
  null,
  (id) => id ?? "",
)

/** A one-time hint the user closed (`useDismissedHint`). */
export const hintDismissed = (slug: string) => flagSetting(`hint:${slug}:dismissed`)

/** The welcome screen, closed for good. */
export const welcomeDismissed = flagSetting("wizard.dismissed")

/** What the ⌘K palette opened lately, newest first. */
export const paletteRecents = jsonSetting<RecentItem[]>(
  "palette.recents",
  (value): value is RecentItem[] => isArray(value),
  [],
)

/** Muted "Needs attention" items: item key → muted until (ms). */
export const attentionMutes = jsonSetting<Record<string, number>>(
  "attention_mutes",
  (value): value is Record<string, number> =>
    typeof value === "object" && value !== null && !isArray(value),
  {},
)

export const notificationThresholds = jsonSetting<NotificationThreshold[]>(
  "notifications.thresholds",
  (value): value is NotificationThreshold[] => isArray(value),
  [],
)

/**
 * The AI features switch, on unless set to `"false"`. Written by
 * `set_ai_features`, which also seals it, never by a plain kv write.
 */
export const aiEnabled = defineSetting<boolean>("ai_enabled", (raw) => raw !== "false", true)

export const aiShareMode = defineSetting<ShareMode>(
  "ai_share_mode",
  (raw) => (isShareMode(raw) ? raw : undefined),
  DEFAULT_SHARE_MODE,
)

/** Field names masked in shared job data (comma separated, `*` wildcards). */
export const aiSharePatterns = defineSetting<string>(
  "ai_share_patterns",
  (raw) => raw,
  DEFAULT_SHARE_PATTERNS,
)

/** Whether agent tool calls are recorded in the activity log. */
export const mcpActivityEnabled = defineSetting<boolean>(
  "mcp_activity_enabled",
  (raw) => raw !== "false",
  true,
  (on) => (on ? "true" : "false"),
)

export const askAiDefaultAgent = defineSetting<AskAgent>(
  "ask_ai_default_agent",
  (raw) => (isAskAgent(raw) ? raw : undefined),
  "claudeCode",
)

/** A group's code folder, handed to the agent; `null` until set. */
export const askAiCodeFolder = (groupId: string) =>
  defineSetting<string | null>(
    `ask_ai_folder:${groupId}`,
    (raw) => raw.trim() || null,
    null,
    (folder) => (folder ?? "").trim(),
  )
