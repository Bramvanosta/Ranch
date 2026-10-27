/**
 * SSH tunnel payload helpers.
 *
 * Two responsibilities:
 *   1. `buildSshTunnelPayload` — translate the flat form values into the
 *      `SshTunnelConfig` boundary shape that the backend expects on the
 *      `addConnection` / `updateConnection` / `testConnection` calls. Returns
 *      `null` when SSH is disabled (defensive — prevents stale form state
 *      from leaking into a SSH-disabled save, mirroring the TLS-CA pattern).
 *   2. `parseSshTunnelDefaults` — inverse: take a `Connection.sshTunnel`
 *      from the backend (or `null`) and return the flat default values for
 *      the form. Used by edit mode pre-population.
 *
 * The boundary type does NOT carry the SSH password — the frontend learns
 * "a password is set" from `auth.kind === "password"` and renders the
 * "Leave blank to keep current" placeholder accordingly. The actual
 * password lives in the OS keychain at `{connection_id}::ssh`.
 */

import type { Connection, SshTunnelConfig } from "@/lib/tauri/bindings"

import type { ConnectionFormInput } from "./connectionFormSchema"

/**
 * Accepts the form's INPUT type (`ConnectionFormInput`) — that's what
 * `form.getValues()` returns and what `handleSubmit`'s callback sees post-
 * coerce. Optional fields are coalesced to sensible defaults so the
 * output `SshTunnelConfig` always has typed strings/numbers.
 */
export function buildSshTunnelPayload(values: ConnectionFormInput): SshTunnelConfig | null {
  if (!values.ssh_enabled) return null
  return {
    host: values.ssh_host ?? "",
    port: values.ssh_port ?? 22,
    user: values.ssh_user ?? "",
    auth:
      (values.ssh_auth_kind ?? "keyFile") === "keyFile"
        ? { kind: "keyFile", keyFilePath: values.ssh_key_file_path ?? "" }
        : { kind: "password" },
    jumpHost: values.ssh_jump_enabled
      ? {
          host: values.ssh_jump_host ?? "",
          port: values.ssh_jump_port ?? 22,
          user: values.ssh_jump_user ?? "",
        }
      : null,
  }
}

export interface SshTunnelDefaults {
  ssh_enabled: boolean
  ssh_host: string
  ssh_port: number
  ssh_user: string
  ssh_auth_kind: "keyFile" | "password"
  ssh_key_file_path: string
  ssh_jump_enabled: boolean
  ssh_jump_host: string
  ssh_jump_port: number
  ssh_jump_user: string
}

/**
 * Reverse of `buildSshTunnelPayload`. Given a connection (or its missing
 * sshTunnel), return the flat form defaults. Used by `ConnectionForm` in
 * edit mode and as the "no SSH" baseline in create mode.
 */
export function parseSshTunnelDefaults(connection: Connection | null): SshTunnelDefaults {
  const cfg = connection?.sshTunnel ?? null
  if (!cfg) {
    return {
      ssh_enabled: false,
      ssh_host: "",
      ssh_port: 22,
      ssh_user: "",
      ssh_auth_kind: "keyFile",
      ssh_key_file_path: "",
      ssh_jump_enabled: false,
      ssh_jump_host: "",
      ssh_jump_port: 22,
      ssh_jump_user: "",
    }
  }
  return {
    ssh_enabled: true,
    ssh_host: cfg.host,
    ssh_port: cfg.port,
    ssh_user: cfg.user,
    ssh_auth_kind: cfg.auth.kind,
    ssh_key_file_path: cfg.auth.kind === "keyFile" ? cfg.auth.keyFilePath : "",
    ssh_jump_enabled: cfg.jumpHost !== null,
    ssh_jump_host: cfg.jumpHost?.host ?? "",
    ssh_jump_port: cfg.jumpHost?.port ?? 22,
    ssh_jump_user: cfg.jumpHost?.user ?? "",
  }
}
