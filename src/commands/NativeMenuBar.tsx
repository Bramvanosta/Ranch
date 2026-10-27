import { useEffect, useRef, useState } from "react"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { toAccelerator } from "./chords"

import { type CommandState, commandState, runCommand, useCommandHandlers } from "./handlers"
import { type MenuEntry, menuEntries, visibleMenus } from "./menuModel"
import type { MenuName } from "./registry"

type MenuApi = typeof import("@tauri-apps/api/menu")
type NativeItem = Awaited<ReturnType<MenuApi["MenuItem"]["new"]>>

/**
 * The desktop app's native menu bar, built
 * from the command registry with `@tauri-apps/api/menu` and set as the app
 * menu (on Linux it attaches to each window). Clicks and accelerators run the
 * registry command; items grey out and relabel as handlers come and go.
 *
 * The platform's own items (About, Hide, Quit, Undo…Select All, Minimize,
 * Zoom, Full Screen, Close) are the predefined ones, so they behave natively.
 */
export function NativeMenuBar() {
  const handlers = useCommandHandlers((s) => s.handlers)
  const items = useRef(new Map<string, NativeItem>())
  const shown = useRef(new Map<string, CommandState>())
  const typing = useTypingInField()

  // Turning AI features off or on rebuilds the menu bar
  // (the Agents menu comes and goes).
  const aiOn = useAiEnabled()

  // The menu bar is the app's, not a window's: each window
  // installs its own menu — its own handlers — when it gets the focus.
  // biome-ignore lint/correctness/useExhaustiveDependencies: rebuilt when AI features change
  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    const install = async () => {
      const api = await import("@tauri-apps/api/menu")
      items.current.clear()
      shown.current.clear()
      const menu = await buildMenu(api, items.current, shown.current)
      if (!cancelled) await menu.setAsAppMenu()
    }
    void install()
    void import("@tauri-apps/api/window").then(async ({ getCurrentWindow }) => {
      const off = await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
        if (focused) void install()
      })
      if (cancelled) off()
      else unlisten = off
    })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [aiOn])

  // Grey out / relabel: only the items whose state changed, once things settle.
  useEffect(() => {
    const t = setTimeout(() => {
      for (const [id, item] of items.current) {
        const next = commandState(handlers, id)
        // ⌘⌫ is "delete to line start" in a text field: the menu must not take it.
        if (typing && id === "job.remove") next.enabled = false
        const prev = shown.current.get(id)
        if (prev?.enabled !== next.enabled) void item.setEnabled(next.enabled)
        if (prev?.label !== next.label) void item.setText(next.label)
        shown.current.set(id, next)
      }
    }, 50)
    return () => clearTimeout(t)
  }, [handlers, typing])

  return null
}

async function buildMenu(
  api: MenuApi,
  items: Map<string, NativeItem>,
  shown: Map<string, CommandState>,
) {
  const { Menu, MenuItem, PredefinedMenuItem, Submenu } = api
  const sep = () => PredefinedMenuItem.new({ item: "Separator" })
  type Predefined = NonNullable<Parameters<typeof PredefinedMenuItem.new>[0]>["item"]
  const predefined = (item: Predefined) => PredefinedMenuItem.new({ item })

  const commandItem = async (id: string, shortcut?: string) => {
    const state = commandState(useCommandHandlers.getState().handlers, id)
    const item = await MenuItem.new({
      id,
      text: state.label,
      enabled: state.enabled,
      accelerator: shortcut ? toAccelerator(shortcut) : undefined,
      action: () => runCommand(id, "menu"),
    })
    items.set(id, item)
    shown.set(id, state)
    return item
  }

  const entries = async (list: MenuEntry[]) =>
    Promise.all(
      list.map(async (e) => {
        if (e.kind === "separator") return sep()
        if (e.kind === "command") return commandItem(e.def.id, e.def.shortcut)
        return Submenu.new({
          text: e.label,
          items: await Promise.all(e.defs.map((d) => commandItem(d.id, d.shortcut))),
        })
      }),
    )

  const before: Partial<Record<MenuName, () => Promise<unknown[]>>> = {
    Ranch: async () => [
      await predefined({ About: { name: "Ranch", version: __APP_VERSION__ } }),
      await sep(),
    ],
    Edit: async () => [
      await predefined("Undo"),
      await predefined("Redo"),
      await sep(),
      await predefined("Cut"),
      await predefined("Copy"),
      await predefined("Paste"),
      await predefined("SelectAll"),
      await sep(),
    ],
    Window: async () => [await predefined("Minimize"), await predefined("Maximize"), await sep()],
  }
  const after: Partial<Record<MenuName, () => Promise<unknown[]>>> = {
    Ranch: async () => [
      await sep(),
      await predefined("Services"),
      await sep(),
      await predefined("Hide"),
      await predefined("HideOthers"),
      await predefined("ShowAll"),
      await sep(),
      await predefined("Quit"),
    ],
    File: async () => [await sep(), await predefined("CloseWindow")],
    View: async () => [await sep(), await predefined("Fullscreen")],
  }

  const submenus = await Promise.all(
    visibleMenus().map(async (name) =>
      Submenu.new({
        text: name,
        items: [
          ...((await before[name]?.()) ?? []),
          ...(await entries(menuEntries(name))),
          ...((await after[name]?.()) ?? []),
        ] as never[],
      }),
    ),
  )
  return Menu.new({ items: submenus })
}

/** Whether the keyboard focus is in a text field right now. */
function useTypingInField(): boolean {
  const [typing, setTyping] = useState(false)
  useEffect(() => {
    const update = () => {
      const el = document.activeElement
      setTyping(
        el instanceof HTMLElement &&
          (el.matches("input, textarea, select") || el.isContentEditable),
      )
    }
    document.addEventListener("focusin", update)
    document.addEventListener("focusout", update)
    return () => {
      document.removeEventListener("focusin", update)
      document.removeEventListener("focusout", update)
    }
  }, [])
  return typing
}
