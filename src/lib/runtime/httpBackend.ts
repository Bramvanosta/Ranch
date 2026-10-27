import { type ServerInfo, setServerInfo } from "./runtime"
import { type AppInvoke, installTauriShim } from "./tauriShim"

/**
 * The server image's backend: `invoke(command, args)` becomes
 * `POST /api/invoke/{command}` on the origin that served the page
 * (`ranch-server`). Same commands, same arguments, same typed errors: the
 * bindings' `typedError` wrapper turns a rejection into
 * `{ status: "error", error }`, so a failure rejects with the server's
 * `RanchErrorDto` (`kind`, `message`, `docUrl`) exactly as Tauri would.
 *
 * Authentication is the server's business — HTTP Basic (the browser attaches
 * the credentials by itself) or an OIDC session cookie. With OIDC, a 401
 * carries a `loginUrl`: the session ended, so the page goes and signs in
 * again, and comes back where it was. The `x-ranch-request` header is what
 * makes the API refuse a request forged from another site.
 */
export const CSRF_HEADER = "x-ranch-request"

export const httpInvoke: AppInvoke = async (command, args) => {
  let response: Response
  try {
    response = await fetch(`/api/invoke/${encodeURIComponent(command)}`, {
      method: "POST",
      credentials: "same-origin",
      headers: { "content-type": "application/json", [CSRF_HEADER]: "1" },
      body: JSON.stringify(args ?? {}),
    })
  } catch (cause) {
    throw {
      kind: "serverUnreachable",
      message: `Cannot reach the Ranch server: ${cause instanceof Error ? cause.message : String(cause)}`,
      docUrl: null,
    }
  }

  const text = await response.text()
  const body = text === "" ? null : safeJson(text)
  if (response.ok) return body

  // Signed out (OIDC): sign in again, and come back here.
  if (response.status === 401 && body && typeof body === "object" && "loginUrl" in body) {
    redirectToLogin(String((body as { loginUrl: unknown }).loginUrl))
  }
  // A `RanchErrorDto` from the server — or whatever stood in its way (a
  // reverse proxy's 502 page, an expired session in front of it…).
  if (body && typeof body === "object" && "kind" in body && "message" in body) throw body
  throw {
    kind: response.status === 401 ? "unauthorized" : "http",
    message:
      response.status === 401
        ? "The Ranch server asks for the password again — reload the page."
        : `The Ranch server answered ${response.status} ${response.statusText}`.trim(),
    docUrl: null,
  }
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

let redirecting = false
function redirectToLogin(loginUrl: string): void {
  // Every query on screen fails at once when a session ends: one redirect.
  if (redirecting || !loginUrl.startsWith("/")) return
  redirecting = true
  const here = window.location.pathname + window.location.search
  window.location.assign(`${loginUrl}?return_to=${encodeURIComponent(here)}`)
}

export function installHttpBackend(): void {
  installTauriShim(httpInvoke)
}

/** Who is signed in, and how — asked once, at boot. Never blocks the app. */
export async function loadServerInfo(): Promise<void> {
  try {
    const response = await fetch("/api/runtime", {
      credentials: "same-origin",
      headers: { [CSRF_HEADER]: "1" },
    })
    if (response.ok) setServerInfo((await response.json()) as ServerInfo)
  } catch {
    // The status bar does without.
  }
}
