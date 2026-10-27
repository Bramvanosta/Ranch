/**
 * Ranch outside Tauri: a stand-in for `window.__TAURI_INTERNALS__`.
 *
 * Everything the frontend asks of the backend goes through one function,
 * Tauri's `invoke` — the typed bindings, and the Tauri JS plugins (window,
 * events, notifications, deep link) alike. Outside a Tauri webview that
 * object does not exist and the app cannot even mount. This installs one:
 *
 *   - **app commands** go to the `invoke` it is given — the HTTP backend of
 *     the server image (`httpBackend.ts`), or the in-memory world of the dev
 *     preview (`src/dev/webPreview.ts`);
 *   - **plugin calls** are answered inertly: there is no native window to
 *     show, no OS deep link, no event bus. Notifications fall through to the
 *     browser's own `Notification` (the plugin's JS already uses it).
 */
export type InvokeArgs = Record<string, unknown>
export type AppInvoke = (command: string, args: InvokeArgs) => Promise<unknown>

export function installTauriShim(invoke: AppInvoke): void {
  let callbackId = 0
  const target = window as unknown as Record<string, unknown>
  target.__TAURI_INTERNALS__ = {
    transformCallback: () => ++callbackId,
    unregisterCallback: () => {},
    convertFileSrc: (path: string) => path,
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main", windowLabel: "main" },
    },
    invoke: async (command: string, args: InvokeArgs = {}) => {
      if (command.startsWith("plugin:")) return answerPlugin(command, ++callbackId)
      return invoke(command, args)
    },
  }
  target.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }
}

function answerPlugin(command: string, id: number): unknown {
  // `listen()` wants an id back; nothing will ever be emitted on it.
  if (command === "plugin:event|listen") return id
  if (command === "plugin:notification|is_permission_granted") {
    return typeof Notification !== "undefined" && Notification.permission === "granted"
  }
  return null
}

/** In a Tauri webview (desktop app), as opposed to a plain browser. */
export function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window
}
