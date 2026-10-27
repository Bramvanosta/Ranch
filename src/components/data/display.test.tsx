import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { CodeBlock, CodeToken } from "./CodeBlock"
import { EmptyState } from "./EmptyState"
import { EnvDot, EnvTag } from "./Env"
import { Kbd } from "./Kbd"
import { SectionHeader } from "./SectionHeader"
import { StatePill } from "./StatePill"

// The long-label fixtures every display component must take.
const QUEUE = "webhooks-outbound-retries-eu-west"
const ERROR = 'ValidationError: "email" must be a valid email for user u_4859'

/**
 * jsdom has no layout, so "does not overflow" is checked structurally: the
 * whole text is rendered (nothing cut in JS), and the element carries no
 * class that would clip it. Real rendering is covered by `pnpm check:overflow`.
 */
const CLIPPING = ["truncate", "text-ellipsis", "overflow-hidden", "line-clamp-1", "line-clamp-2"]
function expectUnclipped(el: Element, text: string) {
  expect(el.textContent).toContain(text)
  for (const node of [el, ...el.querySelectorAll("*")]) {
    const classes = String((node as HTMLElement).className ?? "").split(/\s+/)
    for (const c of CLIPPING) expect(classes, `${node.tagName} .${c}`).not.toContain(c)
  }
}

describe("display components", () => {
  it("Kbd: mono key hint, spelled out for screen readers", () => {
    render(<Kbd label="Shift Command U">⇧⌘U</Kbd>)
    const kbd = screen.getByText("Shift Command U").closest("kbd") as HTMLElement
    expect(kbd).not.toBeNull()
    expect(kbd.className).toContain("font-mono")
    expectUnclipped(kbd, "⇧⌘U")
  })

  it("EnvDot: named after its environment, or hidden when the name is beside it", () => {
    const { rerender } = render(<EnvDot color="env-production" label="Production" />)
    expect(screen.getByRole("img", { name: "Environment: Production" })).toHaveStyle({
      backgroundColor: "var(--env-production)",
    })
    rerender(<EnvDot color="env-production" decorative />)
    expect(screen.queryByRole("img")).toBeNull()
  })

  it("EnvTag: shows the whole environment name", () => {
    render(<EnvTag color="env-custom-3" label={QUEUE} />)
    const tag = screen.getByTitle(`Environment: ${QUEUE}`)
    expectUnclipped(tag, QUEUE)
    expect(tag.className).toContain("shrink-0")
  })

  it("StatePill: one look per state, with its name", () => {
    render(<StatePill state="waitingChildren" />)
    const pill = screen.getByTitle("State: waiting-children")
    expectUnclipped(pill, "waiting-children")
    expect(pill.className).toContain("text-(--state-delayed-text)")
  })

  it("SectionHeader: a heading that wraps a long title, count in mono", () => {
    render(
      <SectionHeader
        title={`Failed jobs in ${QUEUE}`}
        count={222}
        actions={<button type="button">Retry all</button>}
      />,
    )
    const heading = screen.getByRole("heading", { name: /Failed jobs in/ })
    expectUnclipped(heading, QUEUE)
    expect(heading.className).toContain("break-words")
    expect(screen.getByText("222").className).toContain("font-tabular")
  })

  it("EmptyState: a named region with the reason and the next step", () => {
    render(
      <EmptyState
        title={`No failed jobs in ${QUEUE}`}
        description={ERROR}
        action={<button type="button">Add job</button>}
      />,
    )
    const region = screen.getByRole("region", { name: `No failed jobs in ${QUEUE}` })
    expectUnclipped(region, ERROR)
    expect(screen.getByRole("button", { name: "Add job" })).toBeInTheDocument()
  })

  it("CodeBlock: named, mono, long lines scroll instead of being cut", () => {
    render(
      <CodeBlock label="Payload">
        <CodeToken kind="key">"error"</CodeToken>: <CodeToken kind="string">"{ERROR}"</CodeToken>
      </CodeBlock>,
    )
    const pre = screen.getByRole("figure", { name: "Payload" }).querySelector("pre") as HTMLElement
    expect(pre.className).toEqual(expect.stringContaining("overflow-x-auto"))
    expect(pre.className).toContain("font-mono")
    expect(pre.textContent).toContain(ERROR)
    expect(screen.getByText('"error"').className).toBe("text-(--syntax-key)")
  })
})
