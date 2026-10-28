import { expect, test } from "@playwright/test"

import { ROUTES, THEMES } from "../overflow/routes"
import { detectContrast } from "./detect"

// Every screen, in both themes: text reaches WCAG AA.
const SETTLE_MS = 600

const withScenario = (path: string) => {
  const url = new URL(path, "http://preview.local")
  if (!url.searchParams.has("preview")) url.searchParams.set("preview", "full")
  return `${url.pathname}${url.search}`
}

for (const theme of THEMES) {
  test.describe(`contrast ${theme}`, () => {
    test.use({ colorScheme: theme, viewport: { width: 1440, height: 900 } })
    for (const route of ROUTES) {
      test(route.name, async ({ page }) => {
        await page.goto(withScenario(route.path))
        await page.waitForFunction(() => (document.body.innerText || "").trim().length > 0)
        if (route.ready) await page.locator(route.ready).first().waitFor()
        await page.evaluate(() => document.fonts.ready)
        await page.waitForTimeout(SETTLE_MS)
        const found = await page.evaluate(detectContrast)
        expect(
          found.map(
            (v) =>
              `${v.ratio}:1 < ${v.needed} "${v.text}" ${v.color} on ${v.background} (${v.selector})${v.count > 1 ? ` ×${v.count}` : ""}`,
          ),
          "text under WCAG AA",
        ).toEqual([])
      })
    }
  })
}
