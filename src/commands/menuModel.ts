import type { Need } from "@/capabilities"
import { aiFeaturesOn } from "@/lib/ai/aiFeatures"

import { COMMANDS, type CommandDef, MENU_ORDER, type MenuName } from "./registry"

/** One menu as the menu bar and the Ranch button show it: sections of items, submenus in place. */
export type MenuEntry =
  | { kind: "command"; def: CommandDef }
  | { kind: "submenu"; label: string; defs: CommandDef[] }
  | { kind: "separator" }

/** Whether the connection on screen declares a command's capability. */
export type Supports = (need: Need) => boolean

const supportsAll: Supports = () => true

export function menuEntries(menu: MenuName, supports: Supports = supportsAll): MenuEntry[] {
  const defs = COMMANDS.filter(
    (c) => c.menu === menu && (!c.agents || aiFeaturesOn()) && (!c.needs || supports(c.needs)),
  )
  const entries: MenuEntry[] = []
  let section: number | undefined
  const seenSubmenus = new Set<string>()
  for (const def of defs) {
    const s = def.section ?? 0
    if (section !== undefined && s !== section) entries.push({ kind: "separator" })
    section = s
    if (def.submenu) {
      if (seenSubmenus.has(def.submenu)) continue
      seenSubmenus.add(def.submenu)
      entries.push({
        kind: "submenu",
        label: def.submenu,
        defs: defs.filter((d) => d.submenu === def.submenu),
      })
    } else {
      entries.push({ kind: "command", def })
    }
  }
  return entries
}

/** Menus that have at least one command here (Agents disappears with AI off). */
export function visibleMenus(supports: Supports = supportsAll): MenuName[] {
  return MENU_ORDER.filter((m) => menuEntries(m, supports).length > 0)
}
