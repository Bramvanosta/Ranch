import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

const native = vi.hoisted(() => ({
  items: [] as { text?: string; enabled?: boolean; accelerator?: string }[],
  popup: vi.fn(),
}))
vi.mock("@tauri-apps/api/menu", () => ({
  Menu: { new: async () => ({ popup: native.popup }) },
  MenuItem: {
    new: async (opts: { text: string; enabled: boolean; accelerator?: string }) => {
      native.items.push(opts)
      return opts
    },
  },
  PredefinedMenuItem: { new: async () => ({ separator: true }) },
  Submenu: {
    new: async (opts: { text: string }) => {
      native.items.push({ text: `▸ ${opts.text}` })
      return opts
    },
  },
}))

import { type MenuAction, separator, TargetMenu } from "./TargetMenu"

const actions = (run = vi.fn()): MenuAction[] => [
  { label: "Retry 3 Jobs", shortcut: "cmd+shift+r", run },
  { label: "Promote", disabled: true, run },
  separator,
  { kind: "submenu", label: "Move to", items: [{ label: "Production", run }] },
]

describe("TargetMenu", () => {
  afterEach(() => {
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
    native.items.length = 0
    native.popup.mockReset()
  })

  it("in a browser: the in-window menu with the same items", async () => {
    const run = vi.fn()
    render(
      <TargetMenu actions={() => actions(run)} render={<div data-testid="target" />}>
        row
      </TargetMenu>,
    )
    fireEvent.contextMenu(screen.getByTestId("target"))
    fireEvent.click(await screen.findByRole("menuitem", { name: /Retry 3 Jobs/ }))
    expect(run).toHaveBeenCalledTimes(1)
  })

  it("on the desktop: a native menu at the pointer, shortcuts and disabled states included", async () => {
    ;(window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {}
    render(
      <TargetMenu actions={() => actions()} render={<div data-testid="target" />}>
        row
      </TargetMenu>,
    )
    const event = fireEvent.contextMenu(screen.getByTestId("target"))
    expect(event).toBe(false) // the WebView's own menu is suppressed
    await waitFor(() => expect(native.popup).toHaveBeenCalledTimes(1))
    expect(native.items).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          text: "Retry 3 Jobs",
          enabled: true,
          accelerator: expect.stringContaining("R"),
        }),
        expect.objectContaining({ text: "Promote", enabled: false }),
        expect.objectContaining({ text: "▸ Move to" }),
      ]),
    )
    expect(screen.queryByRole("menu")).toBeNull()
  })
})
