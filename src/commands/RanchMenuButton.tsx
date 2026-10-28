import { ChevronDownIcon } from "lucide-react"
import { Kbd } from "@/components/data/Kbd"
import { RanchMark } from "@/components/RanchMark"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useAiEnabled } from "@/lib/ai/aiFeatures"

import { displayChord } from "./chords"
import { commandState, runCommand, useCommandHandlers } from "./handlers"
import { type MenuEntry, menuEntries, visibleMenus } from "./menuModel"
import { useMenuSupports } from "./useMenuSupports"

/**
 * The menu bar of the browser runtimes: a
 * compact "Ranch" button in the sidebar header listing the same registry,
 * one submenu per menu, with the same shortcuts and disabled states.
 */
export function RanchMenuButton() {
  // The Agents menu comes and goes with AI features.
  useAiEnabled()
  // What the connection on screen does not declare is left out.
  const { supports } = useMenuSupports()
  const handlers = useCommandHandlers((s) => s.handlers)
  const item = (id: string, shortcut?: string) => {
    const state = commandState(handlers, id)
    return (
      <DropdownMenuItem
        key={id}
        disabled={!state.enabled}
        onSelect={() => runCommand(id, "ui")}
        className="gap-6"
      >
        <span className="flex-1">{state.label}</span>
        {shortcut ? <Kbd>{displayChord(shortcut)}</Kbd> : null}
      </DropdownMenuItem>
    )
  }
  const render = (entries: MenuEntry[]) =>
    entries.map((e, i) =>
      e.kind === "separator" ? (
        // biome-ignore lint/suspicious/noArrayIndexKey: separators have no identity
        <DropdownMenuSeparator key={`sep-${i}`} />
      ) : e.kind === "command" ? (
        item(e.def.id, e.def.shortcut)
      ) : (
        <DropdownMenuSub key={e.label}>
          <DropdownMenuSubTrigger>{e.label}</DropdownMenuSubTrigger>
          <DropdownMenuSubContent>
            {e.defs.map((d) => item(d.id, d.shortcut))}
          </DropdownMenuSubContent>
        </DropdownMenuSub>
      ),
    )

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          data-testid="ranch-menu"
          className="flex h-control items-center gap-1.5 rounded-control px-2 text-sm font-semibold text-(--text) outline-none hover:bg-(--surf2) focus-visible:ring-2 focus-visible:ring-(--accent)/50"
        >
          <RanchMark size={20} />
          <span className="font-mono">ranch</span>
          <ChevronDownIcon aria-hidden="true" className="size-3.5 text-(--text2)" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-52">
        {visibleMenus(supports).map((menu) => (
          <DropdownMenuSub key={menu}>
            <DropdownMenuSubTrigger>{menu}</DropdownMenuSubTrigger>
            <DropdownMenuSubContent className="min-w-64">
              {render(menuEntries(menu, supports))}
            </DropdownMenuSubContent>
          </DropdownMenuSub>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
