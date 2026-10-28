import type { FakeHandlers } from "./handlers"
import { PreviewError } from "./types"
import { offeredUpdate, world } from "./world"

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
  install_update: () => {
    throw new PreviewError(
      "updater",
      "The web preview has nothing to install; this is what a failed install looks like.",
    )
  },
} satisfies FakeHandlers
