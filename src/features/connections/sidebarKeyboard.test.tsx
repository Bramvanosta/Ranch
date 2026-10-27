import { fireEvent, render, screen } from "@testing-library/react"
import { useState } from "react"
import { describe, expect, it, vi } from "vitest"

import { handleSidebarKeyDown } from "./sidebarKeyboard"

function Node({ label, children }: { label: string; children?: React.ReactNode }) {
  const [open, setOpen] = useState(true)
  return (
    <li data-sidebar-item>
      <div data-sidebar-rowbox>
        {children ? (
          <button
            type="button"
            data-sidebar-toggle
            aria-expanded={open}
            aria-label={`Toggle ${label}`}
            onClick={() => setOpen((v) => !v)}
          />
        ) : null}
        <a href={`#${label}`} data-sidebar-row>
          {label}
        </a>
      </div>
      {children && open ? <ul>{children}</ul> : null}
    </li>
  )
}

function Tree() {
  return (
    <nav onKeyDown={handleSidebarKeyDown}>
      <ul>
        <Node label="prod">
          <Node label="emails" />
          <Node label="images" />
        </Node>
        <Node label="staging" />
      </ul>
    </nav>
  )
}

const row = (name: string) => screen.getByRole("link", { name })

describe("sidebar keyboard", () => {
  it("↑ and ↓ move between rows", () => {
    render(<Tree />)
    row("prod").focus()
    fireEvent.keyDown(row("prod"), { key: "ArrowDown" })
    expect(row("emails")).toHaveFocus()
    fireEvent.keyDown(row("emails"), { key: "ArrowDown" })
    fireEvent.keyDown(row("images"), { key: "ArrowDown" })
    expect(row("staging")).toHaveFocus()
    fireEvent.keyDown(row("staging"), { key: "ArrowUp" })
    expect(row("images")).toHaveFocus()
  })

  it("← folds an open row, → unfolds it", () => {
    render(<Tree />)
    row("prod").focus()
    fireEvent.keyDown(row("prod"), { key: "ArrowLeft" })
    expect(screen.queryByRole("link", { name: "emails" })).toBeNull()
    fireEvent.keyDown(row("prod"), { key: "ArrowRight" })
    expect(row("emails")).toBeInTheDocument()
  })

  it("← on a leaf moves to its parent row", () => {
    render(<Tree />)
    row("images").focus()
    fireEvent.keyDown(row("images"), { key: "ArrowLeft" })
    expect(row("prod")).toHaveFocus()
  })

  it("⌘Enter opens the row in place", () => {
    render(<Tree />)
    const onClick = vi.fn((e: Event) => e.preventDefault())
    row("emails").addEventListener("click", onClick)
    fireEvent.keyDown(row("emails"), { key: "Enter", metaKey: true })
    expect(onClick).toHaveBeenCalledTimes(1)
  })
})
