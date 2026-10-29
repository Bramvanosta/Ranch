/**
 * First-Run Wizard.
 *
 * 2-step onboarding overlay shown when the user has zero saved
 * connections AND hasn't dismissed the wizard yet.
 *
 * Steps:
 *   1. Add connection: opens the two-step connection sheet.
 *   2. MCP installers: the same cards as Agents › Setup & safety.
 *
 * Dismissal persists via `kv_settings` key `wizard.dismissed`.
 */
import { useEffect, useState } from "react"

import { Button } from "@/components/ui/button"
import { fetchConnections } from "@/data/connections/queries"
import { useInvalidateHierarchy } from "@/data/hierarchy"
import { useMcpInstallerStatus } from "@/data/mcp/queries"
import { welcomeDismissed } from "@/data/settings/settings"
import { readSettingOrDefault, writeSetting } from "@/data/settings/useSetting"
import { ConnectionSheet } from "@/features/connections"
import { McpSetupCards } from "@/features/mcp"
import { useSetAiFeatures } from "@/lib/ai/aiFeatures"
import { hasAgents } from "@/lib/runtime/runtime"

export function useFirstRunWizardVisibility() {
  const [show, setShow] = useState(false)
  const [resolved, setResolved] = useState(false)

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const [conns, dismissed] = await Promise.all([
        fetchConnections().catch(() => []),
        readSettingOrDefault(welcomeDismissed),
      ])
      if (cancelled) return
      const hasConnections = conns.length > 0
      setShow(!hasConnections && !dismissed)
      setResolved(true)
    })()
    return () => {
      cancelled = true
    }
  }, [])

  return { show, resolved, setShow }
}

interface FirstRunWizardProps {
  onClose: () => void
}

export function FirstRunWizard({ onClose }: FirstRunWizardProps) {
  const [step, setStep] = useState<1 | 2>(1)
  const [adding, setAdding] = useState(false)
  // Step 2 first asks; the installers come after a Yes.
  const [withAgents, setWithAgents] = useState(false)
  const setAi = useSetAiFeatures()
  const invalidateHierarchy = useInvalidateHierarchy()

  const installerStatus = useMcpInstallerStatus(step === 2 && withAgents)

  const dismiss = async (markDismissed: boolean) => {
    if (markDismissed) {
      await writeSetting(welcomeDismissed, true).catch(() => {
        // Not remembered: the welcome screen shows again at the next launch.
      })
    }
    onClose()
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-(--bg)/95 backdrop-blur"
      data-testid="first-run-wizard"
    >
      <div className="flex max-h-[90vh] w-full max-w-[560px] flex-col gap-6 overflow-y-auto rounded-xl border bg-(--surf) p-8 shadow-lg">
        {/* The app tile next to a real title, and the two steps named. */}
        <header className="flex items-center gap-3">
          <img
            src="/ranch-logomark.png"
            alt=""
            className="size-10 rounded-lg ring-1 ring-(--line)"
            width={40}
            height={40}
          />
          <div className="min-w-0 flex-1">
            <h1 className="text-lg font-semibold leading-tight">Welcome to Ranch</h1>
            <p className="text-xs text-(--text2)">
              {step === 1 ? "First, a Redis connection" : "Then, AI agents, if you want them"}
            </p>
          </div>
          <span
            className="shrink-0 rounded-full bg-(--surf) px-2 py-0.5 text-2xs font-medium text-(--text2)"
            data-testid="wizard-step-indicator"
          >
            {hasAgents() ? `Step ${step} of 2` : "Step 1 of 1"}
          </span>
        </header>

        {step === 1 ? (
          <section className="flex flex-col gap-3">
            <p className="text-sm text-(--text2)">
              Point Ranch at the Redis your BullMQ queues live on. Ranch only reads, until you ask
              it to retry, remove or add a job.
            </p>
            <div className="flex justify-end gap-2 pt-2">
              <Button
                type="button"
                variant="ghost"
                onClick={() => void dismiss(true)}
                data-testid="wizard-skip-connection"
              >
                Later
              </Button>
              <Button
                type="button"
                variant="primary"
                onClick={() => setAdding(true)}
                data-testid="wizard-add-connection"
              >
                Add a connection
              </Button>
            </div>
            <ConnectionSheet
              open={adding}
              onOpenChange={setAdding}
              onSaved={() => {
                setAdding(false)
                // The first save creates the first group + environment too.
                invalidateHierarchy()
                // Step 2 wires agent clients on this machine: desktop only.
                if (hasAgents()) setStep(2)
                else void dismiss(true)
              }}
            />
          </section>
        ) : null}

        {step === 2 && !withAgents ? (
          <section className="flex flex-col gap-4" data-testid="wizard-ai-question">
            <h2 className="text-base font-semibold">Use Ranch with AI agents?</h2>
            <p className="text-sm text-(--text2)">
              Agents such as Claude or Cursor can then read your queues and, with your approval,
              retry or remove jobs. Without them Ranch is the same queue tool, with nothing about AI
              on screen. You can change this later in Settings › AI features.
            </p>
            <div className="grid grid-cols-2 gap-2">
              <Button
                type="button"
                variant="default"
                size="lg"
                disabled={setAi.isPending}
                onClick={() => setAi.mutate(true, { onSuccess: () => setWithAgents(true) })}
                data-testid="wizard-ai-yes"
              >
                Yes
              </Button>
              <Button
                type="button"
                variant="default"
                size="lg"
                disabled={setAi.isPending}
                onClick={() => setAi.mutate(false, { onSuccess: () => void dismiss(true) })}
                data-testid="wizard-ai-no"
              >
                No
              </Button>
            </div>
          </section>
        ) : null}

        {step === 2 && withAgents ? (
          <section className="flex flex-col gap-3">
            <p className="text-sm text-(--text2)">
              Wire Ranch&apos;s MCP server into your agent clients, now or later from Agents › Setup
              & safety.
            </p>
            {installerStatus.data ? (
              <div className="max-h-[50vh] overflow-y-auto pr-1">
                <McpSetupCards
                  columns={1}
                  overview={installerStatus.data}
                  onShowManual={() => {}}
                />
              </div>
            ) : null}
            <div className="flex justify-end gap-2 pt-2">
              <Button
                type="button"
                variant="ghost"
                onClick={() => void dismiss(true)}
                data-testid="wizard-skip-mcp"
              >
                Skip
              </Button>
              <Button
                variant="primary"
                type="button"
                onClick={() => void dismiss(true)}
                data-testid="wizard-done"
              >
                Done
              </Button>
            </div>
          </section>
        ) : null}
      </div>
    </div>
  )
}
