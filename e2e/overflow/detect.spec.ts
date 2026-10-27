import { readFileSync } from "node:fs"
import { expect, test } from "@playwright/test"
import { detectOverflow } from "./detect"

const fixture = readFileSync(new URL("./fixture.html", import.meta.url), "utf8")

test.use({ viewport: { width: 800, height: 2000 } })

test("each rule fires on its bad element, and nothing else is reported", async ({ page }) => {
  await page.setContent(fixture)
  const found = await page.evaluate(detectOverflow)
  const summary = found.map((v) => `${v.kind} ${v.text.split(" ")[0]} x${v.count}`).sort()
  expect(summary).toEqual(
    [
      "clip clip-clamp x1",
      "clip clip-ellipsis x1",
      "clip clip-hidden x1",
      "clip dup x2",
      "outside outside x1",
      "spill spill x1",
    ].sort(),
  )
})

test("the window edge counts: a box below the fold is outside", async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 100 })
  await page.setContent(
    `<body style="margin:0"><div style="height:400px">tall block below the fold</div></body>`,
  )
  const found = await page.evaluate(detectOverflow)
  expect(found.map((v) => v.kind)).toContain("outside")
})

test("a spilling cell is reported once, not once per nowrap ancestor", async ({ page }) => {
  await page.setContent(
    `<div style="white-space:nowrap;width:200px"><div style="width:100px">` +
      `<span>a cell label far too long for its narrow column</span></div></div>`,
  )
  const found = await page.evaluate(detectOverflow)
  expect(found).toHaveLength(1)
  expect(found[0]?.kind).toBe("spill")
  expect(found[0]?.selector).toContain("div > div")
})
