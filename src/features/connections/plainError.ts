import type { Connection, TestConnectionError } from "@/lib/tauri/bindings"

/**
 * Why a connection cannot be reached, in plain words: what the
 * row of the home table says instead of "Connection refused (os error 61)".
 * The raw message stays one hover away.
 */
export function plainConnectionError(
  message: string,
  connection: Pick<Connection, "host" | "port">,
): string {
  const m = message.toLowerCase()
  if (/refused|econnrefused|os error 61|os error 111/.test(m)) {
    return `Connection refused. Is Redis running on port ${connection.port}?`
  }
  if (/timed out|timeout|etimedout|os error 60|os error 110/.test(m)) {
    return `No answer from ${connection.host}:${connection.port}. Is the host reachable (VPN, firewall)?`
  }
  if (/noauth|wrongpass|invalid password|authentication|auth failed/.test(m)) {
    return "Redis refused the user or password. Edit the connection to fix it."
  }
  if (/resolve|dns|nodename nor servname|name or service not known|failed to lookup/.test(m)) {
    return `The host name ${connection.host} does not resolve.`
  }
  if (/certificate|tls|ssl|handshake/.test(m)) {
    return "The TLS handshake failed. Check the TLS settings (a custom CA?)."
  }
  if (/ssh/.test(m)) return "The SSH tunnel could not be opened. Is the bastion reachable?"
  if (/keychain/.test(m)) return "The password could not be read from the keychain. Re-enter it."
  return message
}

/**
 * A failed connection test, in plain words: a headline and,
 * when it adds something, the server's own message under it.
 */
export function plainTestError(
  error: TestConnectionError | null,
  connection: Pick<Connection, "host" | "port">,
): { headline: string; detail: string | null } {
  if (!error) return { headline: "The test failed without saying why.", detail: null }
  const at = `${connection.host}:${connection.port}`
  switch (error.kind) {
    case "authFailed":
      return {
        headline: "Redis refused the user or password. Check them, or leave the user blank.",
        detail: null,
      }
    case "timeout":
      return {
        headline: `No answer from ${at} within 4 seconds. Is the host reachable (VPN, firewall)?`,
        detail: null,
      }
    case "cannotReach":
    case "other": {
      const headline = plainConnectionError(error.reason, connection)
      // The server's words only when they say more than the headline.
      const says = error.reason.toLowerCase()
      return { headline, detail: headline.toLowerCase().includes(says) ? null : error.reason }
    }
    case "tlsHandshake":
      return {
        headline: "The TLS handshake failed. Does the server use TLS, and is the CA right?",
        detail: error.reason,
      }
    case "sshTunnel":
      return {
        headline: "The SSH tunnel could not be opened. Check the bastion, its user and key.",
        detail: error.reason,
      }
  }
}
