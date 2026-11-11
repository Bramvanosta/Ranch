/**
 * The sidebar footer's widget surfacing the auto-update state (desktop
 * app only).
 *
 * Three display states:
 *   - **Disabled** (env var, CLI flag, or a build that cannot update itself,
 *     a Linux package install): muted "Updates off" with the reason as tooltip.
 *   - **Available**: an "Update available · vX" button. It opens a confirmation
 *     with the release notes; *Install and restart* downloads the update,
 *     verifies its signature, replaces the app and restarts it, the sheet
 *     showing each step (`InstallUpdateSheet`). Nothing is ever installed
 *     without that click.
 *   - **Idle**: nothing rendered (no badge in the common case).
 */
import { DownloadIcon } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"
import { useCommandHandler } from "@/commands/handlers"
import { useUpdateCheck } from "@/data/updater/queries"
import { InstallUpdateSheet } from "./InstallUpdateSheet"

export function UpdaterStatusBadge() {
  const [confirming, setConfirming] = useState(false)
  const query = useUpdateCheck()
  // Ranch › Check for Updates…: ask now, and say what came back.
  useCommandHandler("app.checkUpdates", () => {
    void query.refetch().then((r) => {
      if (r.data?.available) setConfirming(true)
      else if (r.isError) toast.error("Could not check for updates")
      else toast.success("Ranch is up to date")
    })
  })

  if (!query.data) return null

  if (query.data.disabled) {
    return (
      <span
        className="text-2xs text-(--text2)"
        data-testid="updater-status-disabled"
        title={query.data.disabledReason ?? "Updates disabled"}
      >
        Updates off
      </span>
    )
  }

  if (!query.data.available) return null

  const { version, currentVersion, notes } = query.data
  return (
    <>
      <button
        type="button"
        onClick={() => setConfirming(true)}
        className="inline-flex items-center gap-1 rounded bg-(--state-delayed)/15 px-1.5 py-0.5 text-2xs font-medium text-(--state-delayed-text) hover:bg-(--state-delayed)/25"
        data-testid="updater-status-available"
      >
        <DownloadIcon className="size-3" aria-hidden="true" />
        Update available · v{version}
      </button>
      <InstallUpdateSheet
        open={confirming}
        onOpenChange={setConfirming}
        testId="updater-confirm"
        title={`Update Ranch to v${version}?`}
        description={`You are on v${currentVersion}. The update is downloaded, its signature is checked, then Ranch restarts in a few seconds. Connections and settings are kept.`}
      >
        {notes ? (
          <pre
            className="max-h-48 overflow-auto rounded-card border border-(--line) bg-(--field) p-3 text-xs whitespace-pre-wrap"
            data-testid="updater-notes"
          >
            {notes}
          </pre>
        ) : null}
      </InstallUpdateSheet>
    </>
  )
}
