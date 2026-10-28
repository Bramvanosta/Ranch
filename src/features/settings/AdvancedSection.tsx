import { useMutation } from "@tanstack/react-query"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { hintDismissed, paletteRecents, welcomeDismissed } from "@/data/settings/settings"
import { writeSetting } from "@/data/settings/useSetting"
import { announceSettingsChange } from "@/lib/settingsSync"
import { useCommandPaletteStore } from "@/stores/useCommandPaletteStore"
import { SettingRow, SettingsPane } from "./SettingsPane"

/** The hints a user can dismiss (`useDismissedHint` slugs). */
const HINT_SLUGS = ["cross-queue-scope"] as const

/** Settings › Advanced: put back what Ranch remembers for you. */
export function AdvancedSection() {
  const reset = useMutation({
    mutationFn: async ({ writes }: { writes: Array<() => Promise<void>>; done: string }) => {
      for (const write of writes) await write()
    },
    onSuccess: (_, { done }) => {
      announceSettingsChange({ topic: "kv" })
      toast.success(done)
    },
    onError: (e) => toast.error(e instanceof Error ? e.message : "Could not reset"),
  })
  return (
    <SettingsPane
      title="Advanced"
      description="What Ranch remembers for you, to start again from scratch."
      testId="settings-advanced"
    >
      <div className="flex flex-col">
        <SettingRow
          label="Hints you dismissed"
          hint="The tips you closed show again where they apply."
          control={
            <Button
              type="button"
              variant="default"
              disabled={reset.isPending}
              onClick={() =>
                reset.mutate({
                  writes: HINT_SLUGS.map((slug) => () => writeSetting(hintDismissed(slug), false)),
                  done: "Hints show again",
                })
              }
            >
              Show again
            </Button>
          }
        />
        <SettingRow
          label="Recent items in ⌘K"
          hint="The palette forgets what you opened from it."
          control={
            <Button
              type="button"
              variant="default"
              disabled={reset.isPending}
              onClick={() => {
                useCommandPaletteStore.getState().hydrateRecents([])
                reset.mutate({
                  writes: [() => writeSetting(paletteRecents, [])],
                  done: "Recent items cleared",
                })
              }}
            >
              Clear
            </Button>
          }
        />
        <SettingRow
          label="Welcome screen"
          hint="Shown again at the next launch while Ranch has no connection."
          control={
            <Button
              type="button"
              variant="default"
              disabled={reset.isPending}
              onClick={() =>
                reset.mutate({
                  writes: [() => writeSetting(welcomeDismissed, false)],
                  done: "The welcome screen will show again",
                })
              }
            >
              Show again
            </Button>
          }
        />
      </div>
    </SettingsPane>
  )
}
