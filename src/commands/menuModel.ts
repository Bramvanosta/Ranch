import { aiFeaturesOn } from "@/lib/ai/aiFeatures"

import { COMMANDS, type CommandDef, MENU_ORDER, type MenuName } from "./registry"

/** One menu as the menu bar and the Ranch button show it: sections of items, submenus in place. */
export type MenuEntry =
  | { kind: "command"; def: CommandDef }
  | { kind: "submenu"; label: string; defs: CommandDef[] }
  | { kind: "separator" }

export function menuEntries(menu: MenuName): MenuEntry[] {
  const defs = COMMANDS.filter((c) => c.menu === menu && (!c.agents || aiFeaturesOn()))
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
export function visibleMenus(): MenuName[] {
  return MENU_ORDER.filter((m) => menuEntries(m).length > 0)
}
