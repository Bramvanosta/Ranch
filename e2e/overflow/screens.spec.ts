import { expect, test } from "@playwright/test"
import { applyAllowlist, formatViolation, loadAllowlist, saveResult } from "./allowlist"
import { detectOverflow } from "./detect"
import { ROUTES, SIZES, sizeLabel, THEMES } from "./routes"

const entries = loadAllowlist()

// The app polls every second, so the page never goes network-idle: wait for
// fonts, for the screen to render, then for the first poll to settle.
const SETTLE_MS = 600

const withScenario = (path: string) => {
  const url = new URL(path, "http://preview.local")
  if (!url.searchParams.has("preview")) url.searchParams.set("preview", "full")
  return `${url.pathname}${url.search}`
}

for (const size of SIZES) {
  for (const theme of THEMES) {
    test.describe(`${theme} ${sizeLabel(size)}`, () => {
      // The app follows the system theme (next-themes, `defaultTheme="system"`).
      test.use({ colorScheme: theme, viewport: size })

      for (const route of ROUTES) {
        test(route.name, async ({ page }) => {
          const errors: string[] = []
          page.on("pageerror", (e) => errors.push(e.message))

          await page.goto(withScenario(route.path))
          await page.waitForFunction(() => (document.body.innerText || "").trim().length > 0)
          await page.evaluate(() => document.fonts.ready)
          await page.waitForTimeout(SETTLE_MS)

          await expect(page.locator("html")).toHaveClass(
            theme === "dark" ? /\bdark\b/ : /^(?!.*\bdark\b)/,
          )
          expect(errors, "the screen threw while rendering").toEqual([])

          const where = { route: route.name, theme, size: sizeLabel(size) }
          const violations = await page.evaluate(detectOverflow)
          const { blocking, allowed } = applyAllowlist(entries, violations, where)
          saveResult({ ...where, violations, allowed })

          expect(
            blocking.map((v) => formatViolation(v, where)),
            "text that does not fit (add the fix, or an allow-list entry naming the story that will)",
          ).toEqual([])
        })
      }
    })
  }
}
