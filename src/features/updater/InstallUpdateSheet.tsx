import type { ReactNode } from "react"

import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { useInstallUpdate } from "@/data/updater/mutations"
import { INSTALL_STEPS, InstallSteps } from "./InstallSteps"

/**
 * The confirmation before an update installs, which then follows the install
 * to the end: the sidebar notice and Settings › Updates both open it.
 *
 *   - Before the click: what is about to happen (`description`, the notes).
 *   - Once clicked: each step (download with its bytes, signature, install,
 *     restart), the button naming the step under way. It cannot be closed
 *     or clicked again: the app is about to restart.
 *   - On a failure: the step that failed, the error, and *Try again*.
 */
export function InstallUpdateSheet({
  open,
  onOpenChange,
  title,
  description,
  testId,
  children,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description: string
  testId?: string
  /** The release notes. */
  children?: ReactNode
}) {
  const install = useInstallUpdate()
  const failed = install.error !== null
  // Started from here, or from the other place that offers it.
  const followed = install.isPending || failed || (install.running && install.progress !== null)
  const doing = INSTALL_STEPS.find((s) => s.phase === (install.progress?.phase ?? "downloading"))

  const confirmLabel = failed
    ? "Try again"
    : install.isPending
      ? (doing?.doing ?? "Installing")
      : install.running
        ? "Installing"
        : "Install and restart"

  return (
    <ConfirmSheet
      open={open}
      onOpenChange={(next) => {
        if (!next && failed) install.reset()
        onOpenChange(next)
      }}
      busy={install.isPending}
      // An install started from the other place is under way: one at a time.
      disabled={install.running && !install.isPending}
      testId={testId}
      confirmTestId="updater-install"
      title={title}
      description={description}
      onConfirm={install.install}
      confirmLabel={confirmLabel}
    >
      {children}
      {followed ? <InstallSteps progress={install.progress} failed={failed} /> : null}
      {install.error ? (
        <p role="alert" className="text-sm text-(--state-failed-text)" data-testid="updater-error">
          {install.error.message}
        </p>
      ) : null}
    </ConfirmSheet>
  )
}
