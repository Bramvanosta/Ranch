/**
 * **Ranch web**: the app in a plain browser, for UI work and review.
 *
 * `pnpm dev` + http://localhost:1420 outside Tauri has no backend: every
 * `invoke` throws and the app cannot even mount. This module stands in for
 * the backend with a small, **stateful, in-memory BullMQ world** (groups,
 * connections (one unreachable, one locked), queues with jobs in every state,
 * schedulers, a flow, workers, audit entries, agent activity and a pending
 * agent approval) so that every screen, dialog and empty / error state can
 * be looked at, clicked through and screenshotted.
 *
 * Loaded by `main.tsx` only when `import.meta.env.DEV` **and** there is no
 * Tauri runtime; it is a dynamic import, so it is not in production bundles.
 * Writes mutate the in-memory world (retry moves the job, drain empties the
 * lists…); a reload starts over. Nothing here talks to Redis.
 *
 * `?preview=empty` shows a first launch (no connection, the wizard);
 * `?preview=full` brings the demo world back.
 *
 * Helpers on `window.ranchPreview`: `requestApproval(tool?)` queues another
 * agent approval, `reset()` rebuilds the world.
 */
import { installTauriShim } from "@/lib/runtime/tauriShim"

import { agentsHandlers } from "./agents"
import { auditHandlers } from "./audit"
import { connectionsHandlers } from "./connections"
import type { CommandName, FakeHandlers, InvokeArgs } from "./handlers"
import { jobsHandlers } from "./jobs"
import { maintenanceHandlers } from "./maintenance"
import { metricsHandlers } from "./metrics"
import { schedulersHandlers } from "./schedulers"
import { settingsHandlers } from "./settings"
import { PreviewError, type World } from "./types"
import { offerUpdate, requestApproval, resetWorld, world } from "./world"

/** Every command the demo world answers. */
export const handlers: FakeHandlers = {
  ...connectionsHandlers,
  ...settingsHandlers,
  ...jobsHandlers,
  ...maintenanceHandlers,
  ...schedulersHandlers,
  ...metricsHandlers,
  ...auditHandlers,
  ...agentsHandlers,
}

// ── installation ─────────────────────────────────────────────────────────
declare global {
  interface Window {
    ranchPreview?: {
      requestApproval: (tool?: string) => void
      reset: () => void
      world: () => World
    }
  }
}

/** Read once, at start: the router drops search params it does not know. */

export function installFakeBackend() {
  offerUpdate(new URLSearchParams(window.location.search).get("update"))
  // The shim answers the Tauri plugins; app commands come here.
  installTauriShim(async (cmd, args) => {
    const handler = handlers[cmd as CommandName] as ((args: InvokeArgs) => unknown) | undefined
    if (!handler) {
      console.warn(`[ranch web] no preview for \`${cmd}\``, args)
      throw new PreviewError("invalidInput", `\`${cmd}\` is not available in the web preview`)
    }
    // A little latency, so loading states exist.
    await new Promise((resolve) => setTimeout(resolve, 60 + Math.random() * 120))
    try {
      return structuredClone(handler(args as InvokeArgs))
    } catch (error) {
      if (error instanceof PreviewError) {
        throw { kind: error.kind, message: error.message, docUrl: error.docUrl }
      }
      throw error
    }
  })
  window.ranchPreview = {
    requestApproval,
    reset: resetWorld,
    world: () => world,
  }
  console.info(
    "[ranch web] browser preview with an in-memory BullMQ world; `ranchPreview.requestApproval()` to see an agent approval",
  )
}
