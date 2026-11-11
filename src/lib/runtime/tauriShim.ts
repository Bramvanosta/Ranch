/**
 * Ranch outside Tauri: a stand-in for `window.__TAURI_INTERNALS__`.
 *
 * Everything the frontend asks of the backend goes through one function,
 * Tauri's `invoke`: the typed bindings, and the Tauri JS plugins (window,
 * events, notifications, deep link) alike. Outside a Tauri webview that
 * object does not exist and the app cannot even mount. This installs one:
 *
 *   - **app commands** go to the `invoke` it is given: the HTTP backend of
 *     the server image (`httpBackend.ts`), or the in-memory world of the dev
 *     preview (`src/backend/fake/`);
 *   - **plugin calls** are answered inertly: there is no native window to
 *     show, no OS deep link. Notifications fall through to the browser's own
 *     `Notification` (the plugin's JS already uses it);
 *   - **events**: `listen()` registers in a page-local bus, and only
 *     `emitEvent` (the dev preview's simulated update install) emits on it.
 */
export type InvokeArgs = Record<string, unknown>
export type AppInvoke = (command: string, args: InvokeArgs) => Promise<unknown>

type Callback = (message: unknown) => void

/** The callbacks `transformCallback` was handed, by the id it gave back. */
const callbacks = new Map<number, Callback>()
/** Event name → listener id → the id of its callback. */
const listeners = new Map<string, Map<number, number>>()

export function installTauriShim(invoke: AppInvoke): void {
  let callbackId = 0
  const target = window as unknown as Record<string, unknown>
  target.__TAURI_INTERNALS__ = {
    transformCallback: (callback?: Callback) => {
      const id = ++callbackId
      if (callback) callbacks.set(id, callback)
      return id
    },
    unregisterCallback: (id: number) => callbacks.delete(id),
    convertFileSrc: (path: string) => path,
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main", windowLabel: "main" },
    },
    invoke: async (command: string, args: InvokeArgs = {}) => {
      if (command.startsWith("plugin:")) return answerPlugin(command, args, ++callbackId)
      return invoke(command, args)
    },
  }
  target.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: (event: string, id: number) => unlisten(event, id),
  }
}

/** Deliver `payload` to every `listen(event)` of the page, as Tauri would. */
export function emitEvent(event: string, payload: unknown): void {
  for (const [id, callbackId] of listeners.get(event) ?? []) {
    callbacks.get(callbackId)?.({ event, id, payload })
  }
}

function unlisten(event: string, id: number) {
  const callbackId = listeners.get(event)?.get(id)
  if (callbackId !== undefined) callbacks.delete(callbackId)
  listeners.get(event)?.delete(id)
}

function answerPlugin(command: string, args: InvokeArgs, id: number): unknown {
  if (command === "plugin:event|listen") {
    const event = String(args.event)
    const forEvent = listeners.get(event) ?? new Map<number, number>()
    forEvent.set(id, Number(args.handler))
    listeners.set(event, forEvent)
    return id
  }
  if (command === "plugin:event|unlisten") {
    unlisten(String(args.event), Number(args.eventId))
    return null
  }
  if (command === "plugin:notification|is_permission_granted") {
    return typeof Notification !== "undefined" && Notification.permission === "granted"
  }
  return null
}

/** In a Tauri webview (desktop app), as opposed to a plain browser. */
export function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window
}
