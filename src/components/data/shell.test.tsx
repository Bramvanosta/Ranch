import { fireEvent, render, screen } from "@testing-library/react"
import { useState } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { ViewPanel, ViewTabsRoot } from "@/components/data/ViewTabs"

import { Screen } from "@/layouts/Screen"

import { InfoStrip } from "./InfoStrip"
import { Segmented } from "./Segmented"
import { Toolbar } from "./Toolbar"
import { ViewTabs } from "./ViewTabs"

const QUEUE = "webhooks-outbound-retries-eu-west"

describe("shell pieces", () => {
  it("Toolbar: the page heading, a window drag region, a long title wraps", () => {
    render(<Toolbar title={QUEUE} actions={<button type="button">Add job</button>} />)
    const heading = screen.getByRole("heading", { level: 1, name: QUEUE })
    expect(heading.className).toContain("break-all")
    expect(heading.className).not.toContain("truncate")
    expect(heading.closest("header")).toHaveAttribute("data-tauri-drag-region")
    expect(screen.getByRole("button", { name: "Add job" })).toBeInTheDocument()
  })

  it("Toolbar: no second h1 when the content carries the page heading", () => {
    render(<Toolbar title="#1232" titleAs="div" />)
    expect(screen.queryByRole("heading")).toBeNull()
    expect(screen.getByText("#1232")).toBeInTheDocument()
  })

  it("InfoStrip: a named region in mono", () => {
    render(
      <InfoStrip label="Connection">
        <span>redis.prod.lumen.internal:6379</span>
      </InfoStrip>,
    )
    const strip = screen.getByRole("region", { name: "Connection" })
    expect(strip.className).toContain("font-mono")
  })

  it("ViewTabs: tabs with counts drive the screen's panels", () => {
    function Harness() {
      const [view, setView] = useState("overview")
      return (
        <ViewTabsRoot value={view} onValueChange={(v) => setView(String(v))}>
          <ViewTabs
            items={[
              { value: "overview", label: "Overview" },
              { value: "failed", label: "Failed", count: 222, tone: "danger" },
            ]}
          />
          <ViewPanel value="overview">overview panel</ViewPanel>
          <ViewPanel value="failed">failed panel</ViewPanel>
        </ViewTabsRoot>
      )
    }
    render(<Harness />)
    const failed = screen.getByRole("tab", { name: /Failed/ })
    expect(screen.getByText("222").className).toContain("text-(--state-failed-text)")
    fireEvent.click(failed)
    expect(failed).toHaveAttribute("aria-selected", "true")
    expect(screen.getByText("failed panel")).toBeVisible()
  })

  it("Segmented: one choice in a radio group, arrows move it", () => {
    function Harness() {
      const [scope, setScope] = useState<"queue" | "all">("queue")
      return (
        <Segmented
          label="Scope"
          value={scope}
          onChange={setScope}
          options={[
            { value: "queue", label: "This queue" },
            { value: "all", label: "All queues" },
          ]}
        />
      )
    }
    render(<Harness />)
    const group = screen.getByRole("radiogroup", { name: "Scope" })
    expect(screen.getByRole("radio", { name: "This queue" })).toHaveAttribute(
      "aria-checked",
      "true",
    )
    fireEvent.keyDown(group, { key: "ArrowRight" })
    expect(screen.getByRole("radio", { name: "All queues" })).toHaveAttribute(
      "aria-checked",
      "true",
    )
    fireEvent.click(screen.getByRole("radio", { name: "This queue" }))
    expect(screen.getByRole("radio", { name: "This queue" })).toHaveAttribute(
      "aria-checked",
      "true",
    )
  })

  it("Screen: toolbar, tabs and content; the inspector column only when given", () => {
    const { rerender } = render(
      <Screen toolbar={<Toolbar title="emails" />}>
        <p>list</p>
      </Screen>,
    )
    expect(screen.queryByRole("complementary", { name: "Inspector" })).toBeNull()
    rerender(
      <Screen toolbar={<Toolbar title="emails" />} inspector={<p>job 1232</p>}>
        <p>list</p>
      </Screen>,
    )
    expect(screen.getByRole("complementary", { name: "Inspector" })).toHaveTextContent("job 1232")
  })
})

describe("Screen inspector", () => {
  const withWidth = (wide: boolean) =>
    vi.stubGlobal("matchMedia", (q: string) => ({
      matches: wide && q.includes("1280"),
      addEventListener: () => {},
      removeEventListener: () => {},
    }))
  afterEach(() => vi.unstubAllGlobals())

  it("is a resizable column from 1280 px", () => {
    withWidth(true)
    render(
      <Screen toolbar={<Toolbar title="emails" />} inspector={<p>job 1232</p>}>
        <p>list</p>
      </Screen>,
    )
    const aside = screen.getByRole("complementary", { name: "Inspector" })
    expect(aside.className).not.toContain("absolute")
    expect(aside.closest("[data-panel]") ?? aside.parentElement).toBeTruthy()
  })

  it("lies over the list below 1280 px, so the list keeps its width", () => {
    withWidth(false)
    render(
      <Screen toolbar={<Toolbar title="emails" />} inspector={<p>job 1232</p>}>
        <p>list</p>
      </Screen>,
    )
    expect(screen.getByRole("complementary", { name: "Inspector" }).className).toContain("absolute")
  })
})
