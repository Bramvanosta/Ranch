import { z } from "zod"

import { ENV_COLOR_PALETTE, type EnvColorToken } from "./palette"
import { HIERARCHY_NAME_MAX_CHARS, NEW } from "./placement"

const envColorTokens = ENV_COLOR_PALETTE.map((c) => c.token) as [EnvColorToken, ...EnvColorToken[]]

/**
 * Zod schema for the connection wizard form.
 *
 * Field ranges mirror the backend's expectations:
 *   - port: u16 → 1..65535
 *   - db: u8 → 0..15 (Redis default upper bound; can be raised later)
 *   - env_color: must be one of the curated palette tokens
 *
 * Username/password are optional — many local Redis setups have no auth.
 *
 * Placement: `group_id` / `environment_id` hold an existing
 * id or the `NEW` sentinel; the `*_name` fields (and `env_color`) only matter
 * for `NEW`. The cross-field rules live in the `superRefine` below.
 */
const baseSchema = z.object({
  name: z.string().min(1, "Give this connection a name"),
  redis_host: z.string().min(1, "Host is required"),
  redis_port: z
    .number({ message: "Port must be a number" })
    .int("Port must be an integer")
    .min(1, "Port must be ≥ 1")
    .max(65535, "Port must be ≤ 65535"),
  redis_db: z
    .number({ message: "DB index must be a number" })
    .int("DB index must be an integer")
    .min(0, "DB index must be ≥ 0")
    .max(15, "DB index must be ≤ 15"),
  redis_username: z.string().optional().default(""),
  redis_password: z.string().optional().default(""),
  tls: z.boolean(),
  /**
   * Optional CA PEM. Empty string is the persistent shape for
   * "no custom CA"; the backend's `validate_ca_pem` helper normalizes empty
   * → None. Validation of the PEM body is delegated to the backend (single
   * source of truth via `rustls-pemfile`).
   */
  tls_custom_ca: z.string().optional().default(""),
  /**
   * SSH tunnel structural fields. Lenient client-side
   * validation: the backend's `validate_ssh_tunnel_config` is the single
   * source of truth (mirrors the TLS CA / Redis URL pattern).
   *
   * `ssh_password` follows the same `Leave blank to keep current` semantic
   * as `redis_password`.
   */
  ssh_enabled: z.boolean().default(false),
  ssh_host: z.string().optional().default(""),
  ssh_port: z.number({ message: "Port must be a number" }).int().min(1).max(65535).default(22),
  ssh_user: z.string().optional().default(""),
  ssh_auth_kind: z.enum(["keyFile", "password"]).default("keyFile"),
  ssh_key_file_path: z.string().optional().default(""),
  ssh_password: z.string().optional().default(""),
  ssh_jump_enabled: z.boolean().default(false),
  ssh_jump_host: z.string().optional().default(""),
  ssh_jump_port: z.number({ message: "Port must be a number" }).int().min(1).max(65535).default(22),
  ssh_jump_user: z.string().optional().default(""),
  group_id: z.string().default(""),
  group_name: z.string().optional().default(""),
  environment_id: z.string().default(""),
  environment_name: z.string().optional().default(""),
  env_color: z.enum(envColorTokens),
})

function checkName(
  ctx: z.RefinementCtx,
  path: "group_name" | "environment_name",
  raw: string | undefined,
  kind: string,
) {
  const name = (raw ?? "").trim()
  if (!name) {
    ctx.addIssue({ code: "custom", path: [path], message: `Give the ${kind} a name` })
  } else if ([...name].length > HIERARCHY_NAME_MAX_CHARS) {
    ctx.addIssue({
      code: "custom",
      path: [path],
      message: `Name cannot exceed ${HIERARCHY_NAME_MAX_CHARS} characters`,
    })
  }
}

export const connectionFormSchema = baseSchema.superRefine((values, ctx) => {
  if (!values.group_id) {
    ctx.addIssue({ code: "custom", path: ["group_id"], message: "Choose a group" })
  } else if (values.group_id === NEW) {
    checkName(ctx, "group_name", values.group_name, "group")
  }

  // A new group can only hold a new environment.
  const envIsNew = values.group_id === NEW || values.environment_id === NEW
  if (envIsNew) {
    checkName(ctx, "environment_name", values.environment_name, "environment")
  } else if (!values.environment_id) {
    ctx.addIssue({ code: "custom", path: ["environment_id"], message: "Choose an environment" })
  }
})

/**
 * Input shape — what `useForm`'s state holds. Optional fields are typed as
 * `string | undefined` because Zod's `.default("")` only applies during
 * parse/coerce; the form state itself accepts undefined for unfilled inputs.
 */
export type ConnectionFormInput = z.input<typeof connectionFormSchema>

/**
 * Output shape — what `zodResolver` returns post-coerce. Optional fields are
 * narrowed to `string` (defaults applied). This is the type passed to the
 * submit handler.
 */
export type ConnectionFormValues = z.output<typeof connectionFormSchema>
