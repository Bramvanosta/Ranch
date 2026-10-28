import { bullmqConnection, readRedisEndpoint, redisEndpoint } from "@/domain/bullmqEndpoint"
import type {
  BullmqRedisEndpoint,
  Connection,
  EndpointSchema,
  TestConnectionArgs,
} from "@/lib/tauri/bindings"

import type { ConnectionFormInput, ConnectionFormValues } from "./connectionFormSchema"
import type { EnvColorToken } from "./palette"
import { buildPlacementPayload } from "./placement"
import { buildSshTunnelPayload, parseSshTunnelDefaults } from "./sshTunnelPayload"

/** What step 1 holds: everything the test needs, nothing it does not. */
export const STEP_ONE_FIELDS = [
  "redis_host",
  "redis_port",
  "redis_db",
  "redis_username",
  "redis_password",
  "tls",
  "tls_custom_ca",
  "ssh_enabled",
  "ssh_host",
  "ssh_port",
  "ssh_user",
  "ssh_auth_kind",
  "ssh_key_file_path",
  "ssh_password",
  "ssh_jump_enabled",
  "ssh_jump_host",
  "ssh_jump_port",
  "ssh_jump_user",
] as const satisfies ReadonlyArray<keyof ConnectionFormInput>

/** A field's default in the backend's endpoint schema, when it is a number. */
function numberDefault(schema: EndpointSchema | undefined, key: string, fallback: number): number {
  const d = schema?.fields.find((f) => f.key === key)?.default
  return typeof d === "number" ? d : fallback
}

/**
 * The form as it opens: the saved values of an edited connection (passwords
 * left blank, which means "keep the saved one"), or a local Redis with the
 * defaults the backend lists (port 6379, db 0).
 */
export function connectionFormDefaults(
  editing: Connection | null,
  schema?: EndpointSchema,
): ConnectionFormInput {
  const saved = editing ? readRedisEndpoint(editing) : null
  // The SSH password is never round-tripped.
  const sshDefaults = parseSshTunnelDefaults(saved)
  if (editing) {
    return {
      name: editing.name,
      redis_host: saved?.host ?? "",
      redis_port: saved?.port ?? 6379,
      redis_db: saved?.db ?? 0,
      redis_username: saved?.username ?? "",
      redis_password: "",
      tls: saved?.tls ?? false,
      tls_custom_ca: saved?.tlsCustomCa ?? "",
      ...sshDefaults,
      ssh_password: "",
      group_id: editing.groupId,
      group_name: "",
      environment_id: editing.environmentId,
      environment_name: "",
      env_color: editing.envColor as EnvColorToken,
    }
  }
  return {
    name: "",
    redis_host: "",
    redis_port: numberDefault(schema, "port", 6379),
    redis_db: numberDefault(schema, "db", 0),
    redis_username: "",
    redis_password: "",
    tls: false,
    tls_custom_ca: "",
    ...sshDefaults,
    ssh_password: "",
    group_id: "",
    group_name: "",
    environment_id: "",
    environment_name: "",
    env_color: "env-local",
  }
}

/** The SSH password to send: only with password auth, and only when typed. */
function sshPasswordOf(values: ConnectionFormInput): string | null {
  return values.ssh_enabled && values.ssh_auth_kind === "password" && values.ssh_password
    ? values.ssh_password
    : null
}

/**
 * The endpoint the form describes. Blank fields are absent; the PEM only
 * matters with TLS on, so turning TLS off leaves it out.
 */
export function endpointOf(values: ConnectionFormInput): BullmqRedisEndpoint {
  return redisEndpoint({
    host: values.redis_host,
    port: values.redis_port,
    db: values.redis_db,
    username: values.redis_username || null,
    tls: values.tls,
    tlsCustomCa: values.tls ? values.tls_custom_ca || null : null,
    sshTunnel: buildSshTunnelPayload(values),
  })
}

/** `test_connection` for the server in step 1. */
export function testConnectionArgs(
  values: ConnectionFormInput,
  savedConnectionId: string | null,
): TestConnectionArgs {
  return {
    ...bullmqConnection(endpointOf(values)),
    password: values.redis_password || null,
    sshPassword: sshPasswordOf(values),
    // A blank password means the saved one, for the test as for the save.
    savedConnectionId,
  }
}

/**
 * The fields `add_connection` and `update_connection` share: the whole
 * endpoint, and the secrets beside it. An empty password is "keep the saved
 * one" (edit) or "no auth" (create); `locked` left untouched (`null`) is
 * the backend's default (read-only for Production).
 */
export function connectionSaveFields(values: ConnectionFormValues, locked: boolean | null) {
  return {
    name: values.name,
    ...bullmqConnection(endpointOf(values)),
    password: values.redis_password ? values.redis_password : null,
    sshPassword: sshPasswordOf(values),
    locked,
    ...buildPlacementPayload(values),
  }
}
