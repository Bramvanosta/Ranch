import { useMutation, useQuery } from "@tanstack/react-query"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { Button } from "@/components/ui/button"
import { hasUpdater } from "@/lib/runtime/runtime"
import { commands } from "@/lib/tauri/bindings"

import { SettingRow, SettingsPane } from "./SettingsPane"

/**
 * Settings › Updates: the running version, a check on demand,
 * and the install of a published one. Shares its query
 * with the sidebar's update notice. Nothing installs without a click.
 */
export function UpdatesSection() {
  if (!hasUpdater()) {
    return (
      <SettingsPane title="Updates" testId="settings-updates">
        <p className="text-sm text-(--text2)">
          The server image does not update itself: pull a newer tag of the image.
        </p>
      </SettingsPane>
    )
  }
  return <DesktopUpdates />
}

function DesktopUpdates() {
  const [confirming, setConfirming] = useState(false)
  const query = useQuery({
    queryKey: ["updater.check"],
    queryFn: async () => {
      const r = await commands.checkForUpdates()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
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
    onError: (e) => toast.error(e instanceof Error ? e.message : "The update failed"),
  })
  const data = query.data
  const checkedAt = query.dataUpdatedAt
    ? new Date(query.dataUpdatedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : null

  const status = query.isFetching
    ? "Checking for a newer version"
    : query.isError
      ? `Could not check: ${query.error.message}`
      : data?.disabled
        ? "Updates are turned off for this build."
        : data?.available
          ? `Ranch ${data.version} is available.`
          : "Ranch is up to date."

  return (
    <SettingsPane
      title="Updates"
      description="Ranch checks once a day, and installs nothing without your click."
      testId="settings-updates"
    >
      <div className="flex flex-col">
        <SettingRow
          label={data ? `Ranch ${data.currentVersion}` : "This version"}
          hint={
            <span data-testid="settings-update-status">
              {status}
              {checkedAt && !query.isFetching ? ` Last checked at ${checkedAt}.` : ""}
            </span>
          }
          control={
            data?.available ? (
              <Button type="button" variant="primary" onClick={() => setConfirming(true)}>
                Install and restart
              </Button>
            ) : (
              <Button
                type="button"
                variant="default"
                disabled={query.isFetching || data?.disabled === true}
                onClick={() => void query.refetch()}
              >
                Check now
              </Button>
            )
          }
        />
      </div>
      {data?.available && data.notes ? (
        <div className="flex flex-col gap-1">
          <h3 className="text-sm font-medium">What is new in {data.version}</h3>
          <p className="text-sm whitespace-pre-wrap text-(--text2)">{data.notes}</p>
        </div>
      ) : null}
      <ConfirmSheet
        open={confirming}
        onOpenChange={setConfirming}
        title={`Install Ranch ${data?.version ?? ""} and restart?`}
        description="Ranch downloads the update, checks its signature, replaces itself and restarts. Open windows close."
        confirmLabel="Install and restart"
        busy={install.isPending}
        onConfirm={() => install.mutate()}
      />
    </SettingsPane>
  )
}
