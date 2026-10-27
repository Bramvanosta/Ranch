import type { Connection } from "@/lib/tauri/bindings"

/** The connection as a Redis URL — never with its password (that stays in the keychain). */
export function redisUrl(c: Pick<Connection, "tls" | "username" | "host" | "port" | "db">): string {
  const user = c.username ? `${encodeURIComponent(c.username)}@` : ""
  return `redis${c.tls ? "s" : ""}://${user}${c.host}:${c.port}/${c.db}`
}
