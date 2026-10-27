/**
 * RailLayout footer widget surfacing the
 * auto-update state.
 *
 * Three display states:
 *   - **Disabled** (env var, CLI flag, or a build that cannot update itself —
 *     a Linux package install): muted "Updates off" with the reason as tooltip.
 *   - **Available**: an "Update available · vX" button. It opens a confirmation
 *     with the release notes; *Install and restart* downloads the update,
 *     verifies its signature, replaces the app and restarts it. Nothing is
 *     ever installed without that click.
 *   - **Idle**: nothing rendered (no badge in the common case).
 */
import { useMutation, useQuery } from "@tanstack/react-query"
import { DownloadIcon } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"
import { useCommandHandler } from "@/commands/handlers"
import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { commands } from "@/lib/tauri/bindings"

export function UpdaterStatusBadge() {
  const [confirming, setConfirming] = useState(false)
  const query = useQuery({
    queryKey: ["updater.check"],
    queryFn: async () => {
      const r = await commands.checkForUpdates()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    // Background-check on app start, then at most once a day. Offline
    // or an unreachable feed is not worth a retry storm: the next start asks again.
    staleTime: 24 * 60 * 60 * 1000,
    refetchOnWindowFocus: false,
    retry: false,
  })
  const install = useMutation({
    mutationFn: async () => {
      // On success the app restarts and this never resolves.
      const r = await commands.installUpdate()
      if (r.status === "error") throw new Error(r.error.message)
    },
  })
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
      <ConfirmSheet
        open={confirming}
        // While it installs there is nothing to go back to: the app is about to restart.
        onOpenChange={setConfirming}
        busy={install.isPending}
        testId="updater-confirm"
        confirmTestId="updater-install"
        title={`Update Ranch to v${version}?`}
        description={`You are on v${currentVersion}. The update is downloaded, its signature is checked, then Ranch restarts in a few seconds. Connections and settings are kept.`}
        // The sheet stays open to show progress, or what went wrong.
        onConfirm={() => install.mutate()}
        confirmLabel={install.isPending ? "Installing" : "Install and restart"}
      >
        {notes ? (
          <pre
            className="max-h-48 overflow-auto rounded-card border border-(--line) bg-(--field) p-3 text-xs whitespace-pre-wrap"
            data-testid="updater-notes"
          >
            {notes}
          </pre>
        ) : null}
        {install.isError ? (
          <p
            role="alert"
            className="text-sm text-(--state-failed-text)"
            data-testid="updater-error"
          >
            {install.error.message}
          </p>
        ) : null}
      </ConfirmSheet>
    </>
  )
}
