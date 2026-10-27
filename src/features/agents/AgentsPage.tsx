import { SparklesIcon } from "lucide-react"

import { EmptyState } from "@/components/data/EmptyState"
import { Toolbar } from "@/components/data/Toolbar"
import { ViewPanel, ViewTabs, ViewTabsRoot } from "@/components/data/ViewTabs"
import { Button } from "@/components/ui/button"
import { AskAiSetup } from "@/features/askai/AskAiSetup"
import { McpActivityPanel } from "@/features/mcp/activity/McpActivityPanel"
import { usePendingApprovals } from "@/features/mcp/approval/usePendingApprovals"
import { McpSafetySection } from "@/features/mcp/McpSafetySection"
import { AgentsSetup } from "@/features/settings/AgentsSetup"
import { Screen } from "@/layouts/Screen"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { useOpenSettings } from "@/lib/windows/settingsWindow"

import { ApprovalsTab } from "./ApprovalsTab"
import { NotesTab } from "./NotesTab"

export const AGENTS_TABS = ["approvals", "activity", "notes", "setup"] as const
export type AgentsTab = (typeof AGENTS_TABS)[number]

/**
 * The Agents page: approvals, MCP activity, resolution
 * notes and setup & safety in one place, out of Settings. With AI features
 * off it only says so, and where to turn them back on.
 */
export function AgentsPage({ tab, onTab }: { tab: AgentsTab; onTab: (tab: AgentsTab) => void }) {
  const aiOn = useAiEnabled()
  const pending = usePendingApprovals()
  const openSettings = useOpenSettings()

  if (!aiOn) {
    return (
      <Screen toolbar={<Toolbar title="Agents" />}>
        <EmptyState
          title="AI features are off"
          description="Ranch is a queue tool only: no agent can reach it. Turn AI features on in Settings to use agents."
          action={
            <Button variant="default" onClick={() => openSettings("ai")}>
              Open Settings › AI features
            </Button>
          }
        />
      </Screen>
    )
  }

  return (
    <ViewTabsRoot value={tab} onValueChange={(v) => onTab(v as AgentsTab)} className="h-full gap-0">
      <Screen
        toolbar={
          <Toolbar
            title={
              <span className="inline-flex items-center gap-2">
                <SparklesIcon className="size-4 text-(--ai)" aria-hidden="true" />
                Agents
              </span>
            }
          />
        }
        tabs={
          <ViewTabs
            items={[
              {
                value: "approvals",
                label: "Approvals",
                count: pending.length > 0 ? pending.length : undefined,
                testId: "agents-tab-approvals",
              },
              { value: "activity", label: "Activity", testId: "agents-tab-activity" },
              { value: "notes", label: "Notes", testId: "agents-tab-notes" },
              { value: "setup", label: "Setup & safety", testId: "agents-tab-setup" },
            ]}
          />
        }
        contentClassName="overflow-y-auto"
      >
        <div className="flex w-full flex-col">
          <ViewPanel value="approvals">
            <ApprovalsTab />
          </ViewPanel>
          <ViewPanel value="activity">
            <McpActivityPanel />
          </ViewPanel>
          <ViewPanel value="notes">
            <NotesTab />
          </ViewPanel>
          <ViewPanel value="setup" className="flex flex-col gap-8">
            <section className="flex flex-col gap-3">
              <h2 className="text-lg font-semibold">Agent clients</h2>
              <AgentsSetup />
            </section>
            <section className="flex flex-col gap-3" data-testid="settings-mcp-safety">
              <h2 className="text-lg font-semibold">Safety</h2>
              <McpSafetySection />
            </section>
            <AskAiSetup />
          </ViewPanel>
        </div>
      </Screen>
    </ViewTabsRoot>
  )
}
