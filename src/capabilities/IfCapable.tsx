import type { ReactNode } from "react"

import type { Need } from "./descriptor"
import { useCapabilities } from "./useCapabilities"

/** `children` when the backend of `connectionId` declares `need`; nothing otherwise. */
export function IfCapable({
  connectionId,
  need,
  children,
}: {
  connectionId: string
  need: Need
  children: ReactNode
}) {
  return useCapabilities(connectionId).can(need) ? children : null
}
