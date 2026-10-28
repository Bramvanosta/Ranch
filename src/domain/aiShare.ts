/**
 * How much of a job reaches an agent. Mirrors `core/mcp/share.rs`, which
 * applies it to every answer that carries job data.
 */
export type ShareMode = "everything" | "mask" | "names"

export const DEFAULT_SHARE_MODE: ShareMode = "mask"

export const DEFAULT_SHARE_PATTERNS =
  "email, phone, token, password, passwd, secret, credential, apikey, api_key, authorization, cookie, iban, card*"

export function isShareMode(value: unknown): value is ShareMode {
  return value === "everything" || value === "mask" || value === "names"
}
