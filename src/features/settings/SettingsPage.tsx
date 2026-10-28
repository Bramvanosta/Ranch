import {
  BellIcon,
  CircleArrowDownIcon,
  PaletteIcon,
  RefreshCwIcon,
  SlidersHorizontalIcon,
  SparklesIcon,
  WrenchIcon,
} from "lucide-react"
import type { ReactNode } from "react"

import { Switch } from "@/components/ui/switch"
import { NotificationThresholdsTable } from "@/features/notifications"
import { useAiEnabled, useSetAiFeatures } from "@/lib/ai/aiFeatures"
import { hasAgents, hasOverlayTitleBar } from "@/lib/runtime/runtime"
import { cn } from "@/lib/utils"
import type { SettingsTab } from "@/lib/windows/settingsWindow"

import { AdvancedSection } from "./AdvancedSection"
import { AiShareSetting } from "./AiShareSetting"
import { AppearanceSection } from "./AppearanceSection"
import { GeneralSection } from "./GeneralSection"
import { RefreshSection } from "./RefreshSection"
import { SettingRow, SettingsPane } from "./SettingsPane"
import { UpdatesSection } from "./UpdatesSection"

const TABS: Array<{ value: SettingsTab; label: string; icon: ReactNode }> = [
  { value: "general", label: "General", icon: <SlidersHorizontalIcon /> },
  { value: "appearance", label: "Appearance", icon: <PaletteIcon /> },
  { value: "refresh", label: "Refresh", icon: <RefreshCwIcon /> },
  { value: "alerts", label: "Alerts", icon: <BellIcon /> },
  { value: "updates", label: "Updates", icon: <CircleArrowDownIcon /> },
  { value: "advanced", label: "Advanced", icon: <WrenchIcon /> },
  { value: "ai", label: "AI features", icon: <SparklesIcon /> },
]

/** The pages this runtime offers: no Agents in the server image. */
export function settingsTabs() {
  // AI features stays where there could be agents, so it can be turned back on.
  return TABS.filter((t) => t.value !== "ai" || hasAgents())
}

/**
 * Settings like the settings of any Mac app:
 * toolbar tabs, one page each. On the desktop it fills its own window
 * (`windowed`: the toolbar is the title bar); in a browser it is a page.
 */
export function SettingsPage({
  tab,
  onTab,
  windowed = false,
}: {
  tab: SettingsTab
  onTab: (tab: SettingsTab) => void
  windowed?: boolean
}) {
  const tabs = settingsTabs()
  const aiOn = useAiEnabled()
  const current = tabs.find((t) => t.value === tab) ?? (tabs[0] as (typeof tabs)[number])
  return (
    <div className="flex h-full min-h-0 flex-col bg-(--bg)" data-testid="settings-page">
      <header
        data-tauri-drag-region={windowed ? true : undefined}
        className={cn(
          "flex shrink-0 flex-col items-center gap-1 border-b border-(--line) bg-(--panel) px-4 pb-2",
          windowed && hasOverlayTitleBar() ? "pt-2" : "pt-3",
        )}
      >
        {windowed ? (
          <h1 data-tauri-drag-region className="text-sm font-semibold">
            {current.label}
          </h1>
        ) : null}
        <div
          role="tablist"
          aria-label="Settings"
          className="flex flex-wrap justify-center gap-1"
          data-tauri-drag-region={windowed ? true : undefined}
        >
          {tabs.map((t) => {
            const selected = t.value === current.value
            return (
              <button
                key={t.value}
                type="button"
                role="tab"
                aria-selected={selected}
                aria-controls="settings-panel"
                data-testid={`settings-tab-${t.value}`}
                onClick={() => onTab(t.value)}
                className={cn(
                  "flex min-w-16 flex-col items-center gap-0.5 rounded-control px-2 py-1 text-xs text-(--text2) outline-none hover:bg-(--surf2) hover:text-(--text) focus-visible:ring-2 focus-visible:ring-(--accent)/50 [&_svg]:size-5",
                  selected && "bg-(--sel) text-(--text)",
                  t.value === "ai" && aiOn && "[&_svg]:text-(--ai)",
                )}
              >
                {t.icon}
                {t.label}
              </button>
            )
          })}
        </div>
      </header>
      <div
        id="settings-panel"
        role="tabpanel"
        aria-label={current.label}
        className="min-h-0 flex-1 overflow-y-auto"
      >
        <div className="mx-auto flex w-full max-w-3xl flex-col gap-8 px-6 py-6">
          {current.value === "general" ? <GeneralSection /> : null}
          {current.value === "appearance" ? <AppearanceSection /> : null}
          {current.value === "refresh" ? <RefreshSection /> : null}
          {current.value === "alerts" ? (
            <section className="flex flex-col gap-4" data-testid="settings-notifications">
              <NotificationThresholdsTable />
            </section>
          ) : null}
          {current.value === "updates" ? <UpdatesSection /> : null}
          {current.value === "advanced" ? <AdvancedSection /> : null}
          {current.value === "ai" ? <AiFeaturesPane /> : null}
        </div>
      </div>
    </div>
  )
}

/**
 * Settings › AI features: the switch. The agents' setup,
 * activity and safety live on the Agents page.
 */
function AiFeaturesPane() {
  const aiOn = useAiEnabled()
  const setAi = useSetAiFeatures()
  return (
    <div className="flex flex-col gap-8">
      <SettingsPane
        title="AI features"
        description="Ranch works the same without them: AI only adds agents that can read and, with your approval, act on your queues."
        testId="settings-ai"
      >
        <SettingRow
          label="Use Ranch with AI agents"
          hint={
            aiOn
              ? "Agents connected through MCP can read your queues; writes follow the safety rules below."
              : "Off: no agent menu, no agent marks, and every MCP call is refused. Agent client configs are left as they are."
          }
          testId="settings-ai-switch"
          control={
            <Switch
              aria-label="Use Ranch with AI agents"
              checked={aiOn}
              disabled={setAi.isPending}
              onCheckedChange={(on) => setAi.mutate(on)}
            />
          }
        />
      </SettingsPane>
      {aiOn ? <AiShareSetting /> : null}
      {aiOn ? (
        <p className="text-sm text-(--text2)" data-testid="settings-ai-agents-page">
          Agent clients, approvals, activity and the safety rules are on the Agents page, in the
          sidebar.
        </p>
      ) : null}
    </div>
  )
}
