import { render } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { RanchMark } from "./RanchMark"

describe("RanchMark", () => {
  it("is decorative and takes the head bar from the accent", () => {
    const { container } = render(<RanchMark />)
    const svg = container.querySelector("svg")
    expect(svg).toHaveAttribute("aria-hidden", "true")
    expect(svg).toHaveAttribute("viewBox", "0 0 32 32")
    expect(container.querySelector("rect")).toHaveAttribute("fill", "var(--accent)")
  })

  it("switches to the pixel-grid drawing at 16 px and below", () => {
    const { container } = render(<RanchMark size={16} />)
    const svg = container.querySelector("svg")
    expect(svg).toHaveAttribute("viewBox", "0 0 16 16")
    expect(svg).toHaveAttribute("data-variant", "pixel")
  })
})
