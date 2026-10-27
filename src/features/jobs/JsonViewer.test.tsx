import { fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

const copySpy = vi.fn()
vi.mock("@/hooks/useCopyToClipboard", () => ({
  useCopyToClipboard: () => copySpy,
}))

import { JsonViewer } from "./JsonViewer"

afterEach(() => {
  vi.clearAllMocks()
})

describe("JsonViewer", () => {
  it("renders empty objects as a muted literal (no twisty)", () => {
    render(<JsonViewer value={{}} />)
    // No expand/collapse trigger when there are no children.
    expect(screen.queryByTestId("json-viewer-twisty")).not.toBeInTheDocument()
    expect(screen.getByTestId("json-viewer").textContent).toContain("{}")
  })

  it("renders empty arrays as a muted literal", () => {
    render(<JsonViewer value={[]} />)
    expect(screen.queryByTestId("json-viewer-twisty")).not.toBeInTheDocument()
    expect(screen.getByTestId("json-viewer").textContent).toContain("[]")
  })

  it("renders all keys + values for a nested object", () => {
    render(
      <JsonViewer
        value={{
          to: "a@b.c",
          payload: { id: 42, ok: true, tags: ["alpha", "beta"] },
        }}
      />,
    )
    expect(screen.getAllByTestId("json-key")).toHaveLength(5)
    // Strings rendered with the syntax-string token.
    expect(screen.getAllByTestId("json-string").length).toBeGreaterThan(0)
    // Number + boolean rendered as literal tokens.
    const literals = screen.getAllByTestId("json-literal")
    expect(literals.map((n) => n.textContent)).toEqual(expect.arrayContaining(["42", "true"]))
  })

  it("toggling the twisty collapses children", () => {
    render(<JsonViewer value={{ nested: { a: 1 } }} />)
    // Both `nested` and inner `a` keys visible initially.
    expect(screen.getAllByTestId("json-key").length).toBeGreaterThanOrEqual(2)
    const firstTwisty = screen.getAllByTestId("json-viewer-twisty")[0]
    fireEvent.click(firstTwisty)
    // After collapsing the outer node, the inner key is unmounted.
    expect(screen.queryByText('"a"')).not.toBeInTheDocument()
  })

  it("clicking 'Copy as JSON' calls the clipboard hook with stringified value + label", () => {
    render(<JsonViewer value={{ x: 1 }} copyLabel="payload" />)
    fireEvent.click(screen.getByTestId("json-viewer-copy"))
    expect(copySpy).toHaveBeenCalledWith(JSON.stringify({ x: 1 }, null, 2), "payload")
  })

  it("auto-collapses nodes deeper than `collapseDeeperThan`", () => {
    // 4 levels deep, threshold = 2 → level 3+ should start collapsed.
    render(<JsonViewer value={{ a: { b: { c: { d: 1 } } } }} collapseDeeperThan={2} />)
    // Innermost key `d` should not be in the DOM (parent collapsed).
    expect(screen.queryByText('"d"')).not.toBeInTheDocument()
  })
})
