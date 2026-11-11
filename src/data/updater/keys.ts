/** Query keys of the self-updater. */
export const updaterKeys = {
  /** Whether a newer Ranch is published: the sidebar notice and Settings › Updates share it. */
  check: ["updater.check"] as const,
  /** The install: one at a time, wherever it was started. */
  install: ["updater.install"] as const,
}
