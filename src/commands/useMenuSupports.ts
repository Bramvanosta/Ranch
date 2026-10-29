import { useMemo } from "react"
import { useCapabilities } from "@/capabilities"
import { useSelectedConnectionId } from "@/hooks/useSelectedConnectionId"

import type { Supports } from "./menuModel"
import { COMMANDS } from "./registry"

/**
 * What the connection on screen declares, for the menus: a command it does
 * not declare is left out, not greyed out. Everything is offered while no
 * connection is on screen or the kinds of backend are not listed yet (the
 * handlers grey out what does not apply). `key` changes with what is left
 * out, for a menu built once to rebuild.
 */
export function useMenuSupports(): { supports: Supports; key: string } {
  const capabilities = useCapabilities(useSelectedConnectionId())
  return useMemo(() => {
    const supports: Supports = capabilities.descriptor ? capabilities.can : () => true
    const key = COMMANDS.filter((c) => c.needs && !supports(c.needs))
      .map((c) => c.id)
      .join(",")
    return { supports, key }
  }, [capabilities])
}
