/**
 * One value Ranch keeps in `kv_settings`: its stored key, how to read it
 * back and what it is when it was never written (or holds something Ranch
 * no longer understands). Stored values are text; a setting turns them into
 * the type a screen works with, and back.
 */
export interface Setting<T> {
  readonly key: string
  readonly defaultValue: T
  /** Never throws: a missing or unreadable value is the default. */
  parse(raw: string | null): T
  serialize(value: T): string
}

export function defineSetting<T>(
  key: string,
  parse: (raw: string) => T | undefined,
  defaultValue: T,
  serialize: (value: T) => string = String,
): Setting<T> {
  return {
    key,
    defaultValue,
    parse(raw) {
      if (raw === null) return defaultValue
      try {
        return parse(raw) ?? defaultValue
      } catch {
        return defaultValue
      }
    },
    serialize,
  }
}

/** A JSON value; `isValid` rejects what an older or foreign writer left. */
export function jsonSetting<T>(
  key: string,
  isValid: (value: unknown) => value is T,
  defaultValue: T,
): Setting<T> {
  return defineSetting(
    key,
    (raw) => {
      const value: unknown = JSON.parse(raw)
      return isValid(value) ? value : undefined
    },
    defaultValue,
    (value) => JSON.stringify(value),
  )
}
