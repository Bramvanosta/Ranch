import { PencilIcon, RefreshCwIcon, UnplugIcon } from "lucide-react"
import { useState } from "react"

import { Button } from "@/components/ui/button"
import { useRefreshSettings } from "@/features/settings/useRefreshSettings"
import type { Connection } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { EditConnectionDialog } from "./EditConnectionDialog"

/**
 * UI review 2026-09-19 — what a connection page shows when Redis cannot be
 * reached. It used to be one red line per tab ("Failed to load connection
 * metrics: Connection refused"): true, three times over, and no help. Now one
 * screen for the whole connection: what Ranch tried, what the server said,
 * what usually causes it, and the two things to do — try again, or fix the
 * connection's settings.
 */
interface ConnectionUnreachableProps {
  connection: Connection | undefined
  /** The error as Ranch reports it (`Connection refused (os error 61)`…). */
  message: string
  /** `RanchError` kind: tunes the hints (SSH, keychain…). */
  kind?: string
  onRetry: () => void
  /** A request is in flight (the background poll counts): the icon spins. */
  retrying: boolean
}

const HINTS: Record<string, string[]> = {
  sshAuthFailed: [
    "The bastion refused the SSH user, key or password.",
    "Check the SSH settings of this connection.",
  ],
  sshKeyFileError: [
    "The SSH key file could not be read — has it moved, or is it passphrase-protected?",
  ],
  sshTunnelFailed: [
    "The SSH tunnel could not be opened — is the bastion reachable from this network?",
    "A VPN that is down is the usual cause.",
  ],
  keychain: ["The password could not be read from the OS keychain. Re-enter it in the settings."],
  keychainUnavailable: ["The OS keychain is not available. Re-enter the password in the settings."],
}

const DEFAULT_HINTS = [
  "Is Redis running, and is the host and port right?",
  "Is the VPN or tunnel this server needs up?",
  "Did the password or TLS settings change?",
]

export function ConnectionUnreachable({
  connection,
  message,
  kind,
  onRetry,
  retrying,
}: ConnectionUnreachableProps) {
  const [editOpen, setEditOpen] = useState(false)
  const { settings } = useRefreshSettings()
  const hints = (kind && HINTS[kind]) || DEFAULT_HINTS

  return (
    <div
      role="alert"
      data-testid="connection-unreachable"
      className="mx-auto mt-10 flex w-full max-w-xl flex-col items-center gap-4 rounded-xl border border-dashed border-(--line) bg-(--surf)/20 p-8 text-center"
    >
      <span className="flex size-10 items-center justify-center rounded-full bg-(--danger)/10 text-(--state-failed-text)">
        <UnplugIcon className="size-5" aria-hidden="true" />
      </span>
      <div className="flex flex-col gap-1">
        <h2 className="text-base font-semibold">
          Can&apos;t reach {connection?.name ?? "this connection"}
        </h2>
        {connection?.host ? (
          <p className="font-mono text-xs text-(--text2)">
            {connection.host}:{connection.port} · db {connection.db}
          </p>
        ) : null}
      </div>
      <p
        className="w-full rounded-md border border-(--danger)/30 bg-(--danger)/10 px-3 py-2 font-mono text-xs break-words text-(--state-failed-text)"
        data-testid="connection-unreachable-message"
      >
        {message}
      </p>
      <ul className="flex flex-col gap-1 text-left text-xs text-(--text2)">
        {hints.map((hint) => (
          <li key={hint} className="flex gap-2">
            <span aria-hidden="true">·</span>
            {hint}
          </li>
        ))}
      </ul>
      <div className="flex items-center gap-2">
        <Button
          variant="primary"
          type="button"
          size="sm"
          onClick={onRetry}
          data-testid="connection-unreachable-retry"
          className="gap-1.5"
        >
          <RefreshCwIcon
            className={cn("size-3.5", retrying && "animate-spin")}
            aria-hidden="true"
          />
          Try again
        </Button>
        {connection ? (
          <Button
            type="button"
            size="sm"
            variant="default"
            onClick={() => setEditOpen(true)}
            data-testid="connection-unreachable-edit"
            className="gap-1.5"
          >
            <PencilIcon className="size-3.5" aria-hidden="true" />
            Edit connection
          </Button>
        ) : null}
      </div>
      {settings.auto ? (
        <p className="text-2xs text-(--text2)">
          Ranch keeps trying in the background: this page comes back on its own.
        </p>
      ) : null}
      {connection ? (
        <EditConnectionDialog connection={connection} open={editOpen} onOpenChange={setEditOpen} />
      ) : null}
    </div>
  )
}
