import { useEffect, useState } from "react"

/** Whether a CSS media query matches, followed live (false where `matchMedia` is missing). */
export function useMediaQuery(query: string): boolean {
  const get = () => typeof window !== "undefined" && Boolean(window.matchMedia?.(query).matches)
  const [matches, setMatches] = useState(get)
  useEffect(() => {
    const list = window.matchMedia?.(query)
    if (!list) return
    const update = () => setMatches(list.matches)
    update()
    list.addEventListener("change", update)
    return () => list.removeEventListener("change", update)
  }, [query])
  return matches
}
