import { Kbd } from "@/components/data/Kbd"
import { Sheet } from "@/components/ui/sheet"
import { useAiEnabled } from "@/lib/ai/aiFeatures"

import { displayChord } from "./chords"
import { visibleMenus } from "./menuModel"
import { COMMANDS } from "./registry"

/** Help › Keyboard Shortcuts: every registry chord, read from the registry. */
export function ShortcutsDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  // The Agents menu comes and goes with AI features.
  useAiEnabled()
  const menus = visibleMenus()
    .map((menu) => ({
      menu,
      commands: COMMANDS.filter((c) => c.menu === menu && c.shortcut),
    }))
    .filter((m) => m.commands.length > 0)
  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title="Keyboard shortcuts"
      className="w-[min(40rem,calc(100vw-2rem))]"
    >
      <div className="grid gap-5 sm:grid-cols-2" data-testid="shortcuts-list">
        {menus.map(({ menu, commands }) => (
          <section key={menu} aria-label={menu}>
            <h3 className="mb-1.5 text-2xs font-semibold tracking-wide text-(--text2) uppercase">
              {menu}
            </h3>
            <ul className="flex flex-col gap-1">
              {commands.map((c) => (
                <li key={c.id} className="flex items-center justify-between gap-4 text-sm">
                  <span className="min-w-0 break-words">{c.label.replace(/…$/, "")}</span>
                  <Kbd>{displayChord(c.shortcut as string)}</Kbd>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
    </Sheet>
  )
}
