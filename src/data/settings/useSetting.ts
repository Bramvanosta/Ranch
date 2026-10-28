import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useCallback } from "react"

import { unwrap } from "@/data/result"
import { toastFailure } from "@/data/toasts"
import { announceSettingsChange } from "@/lib/settingsSync"
import { commands } from "@/lib/tauri/bindings"

import type { Setting } from "./defineSetting"

/**
 * Every setting's query lives under `["kv", …]`, so a change made in another
 * window (`settingsSync`) reaches it.
 */
export function settingQueryKey<T>(setting: Setting<T>) {
  return ["kv", setting.key] as const
}

/** The stored value, parsed. A failed read throws a `RanchError`. */
export async function readSetting<T>(setting: Setting<T>): Promise<T> {
  return setting.parse(unwrap(await commands.getKvSetting(setting.key)))
}

/** The stored value, or the default when it cannot be read. */
export function readSettingOrDefault<T>(setting: Setting<T>): Promise<T> {
  return readSetting(setting).catch(() => setting.defaultValue)
}

export async function writeSetting<T>(setting: Setting<T>, value: T): Promise<void> {
  unwrap(await commands.setKvSetting(setting.key, setting.serialize(value)))
}

/**
 * A setting as React state. `value` is the default until the stored value
 * is read (and if it cannot be); `stored` stays `undefined` until then, for
 * a screen that must not guess. `set` applies the new value at once, writes
 * it behind, and tells the other windows.
 */
export function useSetting<T>(
  setting: Setting<T>,
  { enabled = true, staleTime = Number.POSITIVE_INFINITY } = {},
) {
  const queryClient = useQueryClient()
  const queryKey = settingQueryKey(setting)
  const query = useQuery({
    queryKey,
    queryFn: () => readSetting(setting),
    enabled,
    staleTime,
    // A local database read that failed once will fail again.
    retry: false,
  })
  const { mutate } = useMutation({
    mutationFn: (value: T) => writeSetting(setting, value),
    onSuccess: () => announceSettingsChange({ topic: "kv" }),
    onError: (err) => toastFailure("Could not save", err),
  })
  // Keyed by the stored key, not the setting object: a setting made per
  // render (`hintDismissed(slug)`) keeps the same `set`.
  const key = setting.key
  const set = useCallback(
    (value: T) => {
      queryClient.setQueryData(["kv", key], value)
      mutate(value)
    },
    [queryClient, mutate, key],
  )
  return { value: query.data ?? setting.defaultValue, stored: query.data, set, query }
}
