import { useEffect, useState } from "react"

/**
 * Returns `value` once it has stopped changing for `delayMs`. Used to keep a
 * text input instant while the query it drives waits for the typing to settle.
 */
export function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value)
  useEffect(() => {
    if (delayMs <= 0) {
      setDebounced(value)
      return
    }
    const id = setTimeout(() => setDebounced(value), delayMs)
    return () => clearTimeout(id)
  }, [value, delayMs])
  return debounced
}
