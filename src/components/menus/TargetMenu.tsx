import { cloneElement, type ReactElement, type ReactNode } from "react"

import { displayChord } from "@/commands/chords"
import { EnvDot } from "@/components/data/Env"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import { hasNativeMenus } from "@/lib/runtime/runtime"

/**
 * One item of a right-click menu, as data so the same list can
 * become a native menu (desktop) or the in-window Base UI menu (browsers).
 */
export type MenuAction =
  | { kind: "separator" }
  | {
      kind?: "item"
      label: string
      run?: () => void
      disabled?: boolean
      /** Destructive: red in the in-window menu (native menus have no colour). */
      danger?: boolean
      /** Registry chord shown next to the item, when it runs the same thing. */
      shortcut?: string
      /** An environment colour token, shown as a dot in the in-window menu. */
      swatch?: string
      testId?: string
    }
  | { kind: "submenu"; label: string; items: MenuAction[] }

export const separator: MenuAction = { kind: "separator" }

/** The desktop app, for real (not jsdom, not a browser). */
const nativeMenus = () => hasNativeMenus() && "__TAURI_INTERNALS__" in window

/**
 * A right-click menu on `render`: native on the desktop
 * (`@tauri-apps/api/menu`, opened at the pointer), the Base UI context menu
 * in the browser runtimes. `actions` is read when the menu opens.
 */
export function TargetMenu({
  actions,
  render,
  children,
}: {
  actions: () => MenuAction[]
  /** The element that receives the right-click (Base UI `render` convention). */
  render: ReactElement<{ onContextMenu?: (e: React.MouseEvent) => void; children?: ReactNode }>
  /** The trigger's content, unless `render` already holds it. */
  children?: ReactNode
}) {
  if (nativeMenus()) {
    return cloneElement(render, {
      ...(children !== undefined ? { children } : {}),
      onContextMenu: (e: React.MouseEvent) => {
        e.preventDefault()
        void popupNative(actions())
      },
    })
  }
  return (
    <ContextMenu>
      <ContextMenuTrigger render={render}>{children}</ContextMenuTrigger>
      <ContextMenuContent>
        <InWindowItems actions={actions()} />
      </ContextMenuContent>
    </ContextMenu>
  )
}

function InWindowItems({ actions }: { actions: MenuAction[] }) {
  return (
    <>
      {actions.map((a, i) => {
        if (a.kind === "separator") {
          // biome-ignore lint/suspicious/noArrayIndexKey: separators have no identity
          return <ContextMenuSeparator key={`sep-${i}`} />
        }
        if (a.kind === "submenu") {
          return (
            <ContextMenuSub key={a.label}>
              <ContextMenuSubTrigger>{a.label}</ContextMenuSubTrigger>
              <ContextMenuSubContent>
                <InWindowItems actions={a.items} />
              </ContextMenuSubContent>
            </ContextMenuSub>
          )
        }
        return (
          <ContextMenuItem
            key={a.label}
            disabled={a.disabled}
            variant={a.danger ? "destructive" : undefined}
            onClick={a.run}
            data-testid={a.testId}
          >
            {a.swatch ? <EnvDot color={a.swatch} decorative className="mr-1" /> : null}
            {a.label}
            {a.shortcut ? (
              <ContextMenuShortcut>{displayChord(a.shortcut)}</ContextMenuShortcut>
            ) : null}
          </ContextMenuItem>
        )
      })}
    </>
  )
}

async function popupNative(actions: MenuAction[]) {
  const { Menu, MenuItem, PredefinedMenuItem, Submenu } = await import("@tauri-apps/api/menu")
  const { toAccelerator } = await import("@/commands/chords")
  const build = async (list: MenuAction[]): Promise<unknown[]> =>
    Promise.all(
      list.map(async (a) => {
        if (a.kind === "separator") return PredefinedMenuItem.new({ item: "Separator" })
        if (a.kind === "submenu") {
          return Submenu.new({ text: a.label, items: (await build(a.items)) as never[] })
        }
        return MenuItem.new({
          text: a.label,
          enabled: !a.disabled && Boolean(a.run),
          // Shown, not bound: the chord itself is the registry's (menu bar / window).
          accelerator: a.shortcut ? toAccelerator(a.shortcut) : undefined,
          action: () => a.run?.(),
        })
      }),
    )
  const menu = await Menu.new({ items: (await build(actions)) as never[] })
  await menu.popup()
}
