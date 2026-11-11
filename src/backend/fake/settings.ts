import { emitEvent } from "@/lib/runtime/tauriShim"
import type { UpdateProgressEvent } from "@/lib/tauri/bindings"
import type { FakeHandlers } from "./handlers"
import { PreviewError } from "./types"
import { installScenario, offeredUpdate, world } from "./world"

/** The kv settings and the updater. */
export const settingsHandlers = {
  // settings
  get_kv_setting: ({ key }) => world.kv.get(String(key)) ?? null,
  set_kv_setting: ({ key, value }) => {
    world.kv.set(String(key), String(value))
    return null
  },
  // `?update=0.3.0` on the first URL offers that version, to review the flow.
  check_for_updates: () => ({
    available: offeredUpdate !== null,
    version: offeredUpdate,
    notes: offeredUpdate
      ? "### Fixed\n- Waiting jobs were missing from the lists.\n\n### Added\n- Recent jobs on the overview pages."
      : null,
    currentVersion: __APP_VERSION__,
    disabled: false,
    disabledReason: null,
  }),
  install_update: () => simulateInstall(),
} satisfies FakeHandlers

/** The size of the simulated download: about what a Ranch bundle weighs. */
const DOWNLOAD_BYTES = 14_680_064
/** Slow enough to read every phase (and for the sheets check to catch the download). */
const DOWNLOAD_MS = 4_000
const TICK_MS = 80

let installing = false
let attempts = 0

const pause = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))
const report = (progress: UpdateProgressEvent) => emitEvent("updater:progress", progress)

/**
 * `install_update` as the desktop app runs it, phase by phase, on the
 * `updater:progress` event. Ranch web has nothing to replace: "restarting"
 * reloads the page, the closest it has to a restart.
 */
async function simulateInstall(): Promise<null> {
  if (installing) {
    throw new PreviewError("updater", "An update is already installing: wait for it to finish")
  }
  installing = true
  attempts += 1
  try {
    const total = installScenario === "unsized" ? null : DOWNLOAD_BYTES
    const ticks = DOWNLOAD_MS / TICK_MS
    for (let tick = 1; tick <= ticks; tick++) {
      await pause(TICK_MS)
      report({ phase: "downloading", received: Math.round((DOWNLOAD_BYTES * tick) / ticks), total })
    }
    report({ phase: "verifying" })
    await pause(700)
    if (installScenario === "fail" && attempts === 1) {
      throw new PreviewError(
        "updater",
        "The update's signature could not be verified: the signature does not match the download",
      )
    }
    report({ phase: "installing" })
    await pause(1_200)
    report({ phase: "restarting" })
    await pause(1_200)
    window.location.reload()
    // A restart does not come back.
    return await new Promise<null>(() => undefined)
  } finally {
    installing = false
  }
}
