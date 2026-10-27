/**
 * Curated environment color palette.
 *
 * Single source of truth for the color tokens accepted by the backend's
 * `add_connection` command and rendered by `EnvironmentColorPicker`. Adding
 * a new color requires:
 *   1. Define `--env-<token>` in `src/styles/globals.css`
 *   2. Append `{ token, name }` here
 *   3. Optionally extend `default_env_name_for_color` in
 *      `src-tauri/src/commands/connections.rs` for a custom default env name
 *
 * Token convention is shared verbatim between frontend and backend.
 */
export type EnvColorToken =
  | "env-production"
  | "env-staging"
  | "env-local"
  | "env-custom-1"
  | "env-custom-2"
  | "env-custom-3"
  | "env-custom-4"
  | "env-custom-5"
  | "env-custom-6"
  | "env-custom-7"

export interface EnvColorOption {
  token: EnvColorToken
  /** Default human label shown next to the swatch. */
  name: string
}

export const ENV_COLOR_PALETTE: readonly EnvColorOption[] = [
  { token: "env-production", name: "Production" },
  { token: "env-staging", name: "Staging" },
  { token: "env-local", name: "Local" },
  { token: "env-custom-1", name: "Custom 1" },
  { token: "env-custom-2", name: "Custom 2" },
  { token: "env-custom-3", name: "Custom 3" },
  { token: "env-custom-4", name: "Custom 4" },
  { token: "env-custom-5", name: "Custom 5" },
  { token: "env-custom-6", name: "Custom 6" },
  { token: "env-custom-7", name: "Custom 7" },
] as const
