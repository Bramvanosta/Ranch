import type { Environment, EnvironmentSelector, Group, GroupSelector } from "@/lib/tauri/bindings"

import { ENV_COLOR_PALETTE, type EnvColorToken } from "./palette"

/**
 * Where a connection lives. The form keeps placement as
 * four flat fields so react-hook-form can watch them individually; this
 * module owns the `NEW` sentinel and the conversion to the backend selectors.
 */

/** `<select>` value meaning "create a new one" (group or environment). */
export const NEW = "__new__"

/** Mirrors `HIERARCHY_NAME_MAX_CHARS` in `commands/hierarchy.rs`. */
export const HIERARCHY_NAME_MAX_CHARS = 64

export interface PlacementValues {
  group_id: string
  group_name: string
  environment_id: string
  environment_name: string
  env_color: EnvColorToken
}

/** Palette default name for a color ("Production", "Local", "Custom 3"…). */
export function defaultEnvNameForColor(token: EnvColorToken): string {
  return ENV_COLOR_PALETTE.find((c) => c.token === token)?.name ?? "Environment"
}

const PALETTE_NAMES = new Set(ENV_COLOR_PALETTE.map((c) => c.name))

/**
 * When the user picks another swatch for a NEW environment, the name follows
 * the color only while it is still a palette default (or empty); a name the
 * user typed is never overwritten.
 */
export function shouldFollowColor(currentName: string): boolean {
  const trimmed = currentName.trim()
  return trimmed === "" || PALETTE_NAMES.has(trimmed)
}

export function environmentsOfGroup(environments: Environment[], groupId: string): Environment[] {
  return environments.filter((e) => e.groupId === groupId)
}

/**
 * Initial placement for the create form once groups / environments are known.
 * Preference order: an explicit target ("New connection here"), else the first
 * group and its first environment, else a brand-new "Default" / "Local".
 */
export function initialPlacement(
  groups: Group[],
  environments: Environment[],
  preferred?: { groupId?: string; environmentId?: string },
): Pick<PlacementValues, "group_id" | "group_name" | "environment_id" | "environment_name"> {
  const group = groups.find((g) => g.id === preferred?.groupId) ?? groups[0]
  if (!group) {
    return {
      group_id: NEW,
      group_name: "Default",
      environment_id: NEW,
      environment_name: "Local",
    }
  }
  const envs = environmentsOfGroup(environments, group.id)
  const env = envs.find((e) => e.id === preferred?.environmentId) ?? envs[0]
  return {
    group_id: group.id,
    group_name: "",
    environment_id: env ? env.id : NEW,
    environment_name: env ? "" : "Local",
  }
}

export function buildPlacementPayload(values: PlacementValues): {
  group: GroupSelector
  environment: EnvironmentSelector
} {
  const group: GroupSelector =
    values.group_id === NEW
      ? { kind: "new", name: values.group_name.trim() }
      : { kind: "existing", id: values.group_id }

  // A new group cannot contain an existing environment.
  const envIsNew = values.group_id === NEW || values.environment_id === NEW
  const environment: EnvironmentSelector = envIsNew
    ? { kind: "new", name: values.environment_name.trim(), colorToken: values.env_color }
    : { kind: "existing", id: values.environment_id }

  return { group, environment }
}
