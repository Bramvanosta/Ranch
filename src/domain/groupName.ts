import type { Group } from "@/lib/tauri/bindings"

/** Longest accepted group name; mirrors `GROUP_NAME_MAX_CHARS` in `commands/groups.rs`. */
export const GROUP_NAME_MAX_CHARS = 64

/**
 * Client-side mirror of the backend's name rules, so the inline input can
 * explain a rejection before the round-trip. The backend stays the authority.
 */
export function validateGroupName(
  raw: string,
  groups: Pick<Group, "id" | "name">[],
  excludeId?: string,
): string | null {
  const name = raw.trim()
  if (!name) return "Name cannot be empty"
  if ([...name].length > GROUP_NAME_MAX_CHARS) {
    return `Name cannot exceed ${GROUP_NAME_MAX_CHARS} characters`
  }
  const wanted = name.toLowerCase()
  if (groups.some((g) => g.id !== excludeId && g.name.toLowerCase() === wanted)) {
    return `A group named "${name}" already exists`
  }
  return null
}
