/**
 * Where this bundle is running, decided once, at boot (`main.tsx`):
 *
 *   - `desktop`: inside the Tauri app. Everything is available.
 *   - `server`:  served by `ranch-server` (the Docker image), talking HTTP.
 *     No MCP (agents need the desktop app next to them), no self-update (an
 *     image is updated by pulling a tag).
 *   - `preview`: `pnpm dev` in a browser, on an in-memory fake world.
 *
 * Components ask a capability (`hasAgents()`, `hasNativeWindows()`…)
 * rather than comparing strings: the question is "can this work here?", not
 * "where am I?".
 */
export type Runtime = "desktop" | "server" | "preview"

/** `GET /api/runtime`: what `ranch-server` says about itself and the session. */
export interface ServerInfo {
  runtime: "server"
  version: string
  auth: "oidc" | "basic" | "none"
  user: { name: string | null; email: string | null } | null
  logoutUrl: string | null
  secretsEncrypted: boolean
}

let current: Runtime = "desktop"
let serverInfo: ServerInfo | null = null

export function setServerInfo(info: ServerInfo | null): void {
  serverInfo = info
}

export function getServerInfo(): ServerInfo | null {
  return serverInfo
}

export function setRuntime(runtime: Runtime): void {
  current = runtime
  document.documentElement.dataset.runtime = runtime
}

export function getRuntime(): Runtime {
  return current
}

/** MCP setup, approvals, activity: the desktop app's job (and the preview shows them). */
export function hasAgents(): boolean {
  return current !== "server"
}

export function hasUpdater(): boolean {
  // The preview has one too (a stand-in: `?update=0.3.0` offers a version), so the flow can be reviewed.
  return current !== "server"
}

/**
 * Native windows and tabs (Settings, a job in its own window), labelled by
 * Tauri, and the Tauri event bus between them. A browser opens tabs instead
 * and talks across them over a `BroadcastChannel`.
 */
export function hasNativeWindows(): boolean {
  return current === "desktop"
}

/** A native menu bar and native right-click menus; the browser runtimes draw their own. */
export function hasNativeMenus(): boolean {
  return current === "desktop"
}

/**
 * BullMQ's event streams followed live (`queue:events`): the screens refresh
 * when a queue moves. The server and the preview keep polling.
 */
export function hasLiveEvents(): boolean {
  return current === "desktop"
}

/**
 * Served by `ranch-server`: a signed-in session, and no native About, so the
 * sidebar says which runtime and version this is.
 */
export function isRanchServer(): boolean {
  return current === "server"
}

/**
 * The macOS desktop app draws its own title bar area (`titleBarStyle: Overlay`):
 * the sidebar keeps room for the traffic lights and acts as a drag region.
 * Linux keeps native decorations; the browser runtimes have no window chrome.
 */
export function hasOverlayTitleBar(): boolean {
  return current === "desktop" && /Mac/.test(navigator.userAgent)
}
