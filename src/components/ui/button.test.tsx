import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { Button } from "./button"

const LONG = "Retry every failed job in webhooks-outbound-retries-eu-west"

describe("Button", () => {
  it.each([
    ["default", "bg-(--surf)"],
    ["primary", "bg-(--accent)"],
    ["ai", "bg-(--ai)"],
    ["danger", "bg-(--danger)"],
    ["ghost", "bg-transparent"],
  ] as const)("the %s variant sits on its token", (variant, cls) => {
    render(<Button variant={variant}>Go</Button>)
    expect(screen.getByRole("button", { name: "Go" }).className).toContain(cls)
  })

  it.each([
    ["sm", "h-control-sm"],
    ["default", "h-control"],
    ["lg", "h-control-lg"],
    ["icon-sm", "size-control-sm"],
  ] as const)("size %s is one of the spec's heights", (size, cls) => {
    render(<Button size={size}>Go</Button>)
    expect(screen.getByRole("button").className.split(" ")).toContain(cls)
  })

  it("never wraps, shrinks or truncates its label", () => {
    render(<Button>{LONG}</Button>)
    const classes = screen.getByRole("button").className.split(" ")
    expect(classes).toEqual(expect.arrayContaining(["whitespace-nowrap", "shrink-0"]))
    expect(classes).not.toContain("truncate")
    expect(screen.getByRole("button")).toHaveTextContent(LONG)
  })

  it("shows a shortcut hint in mono after the label", () => {
    render(<Button shortcut="⇧⌘R">Retry</Button>)
    const button = screen.getByRole("button")
    const hint = button.querySelector('[data-slot="button-shortcut"]')
    expect(hint).toHaveTextContent("⇧⌘R")
    expect(hint?.className).toContain("font-mono")
    expect(button.textContent).toBe("Retry⇧⌘R")
  })

  it("focus rings use the accent at 50 %", () => {
    render(<Button>Go</Button>)
    expect(screen.getByRole("button").className).toContain("focus-visible:ring-(--accent)/50")
  })
})
