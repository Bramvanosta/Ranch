import { expect, test } from "@playwright/test"

import { ROUTES } from "../overflow/routes"

// With AI features off, no screen shows anything of AI:
// no element drawn with the `--ai` tokens, and no ✦ glyph.
const SETTLE_MS = 600

const withAiOff = (path: string) => {
  const url = new URL(path, "http://preview.local")
  if (!url.searchParams.has("preview")) url.searchParams.set("preview", "full")
  url.searchParams.set("ai", "off")
  return `${url.pathname}${url.search}`
}

test.use({ colorScheme: "dark", viewport: { width: 1440, height: 900 } })

for (const route of ROUTES) {
  test(`AI off: ${route.name}`, async ({ page }) => {
    await page.goto(withAiOff(route.path))
    await page.waitForFunction(() => (document.body.innerText || "").trim().length > 0)
    await page.waitForTimeout(SETTLE_MS)

    const found = await page.evaluate(() => {
      const ai = [...document.querySelectorAll<HTMLElement | SVGElement>("[class], [style]")]
        .filter((el) => {
          const cls = el.getAttribute("class") ?? ""
          const style = el.getAttribute("style") ?? ""
          return /\(--ai[\w-]*\)/.test(cls) || /var\(--ai[\w-]*\)/.test(style)
        })
        .map((el) => `${el.tagName.toLowerCase()} "${(el.textContent ?? "").trim().slice(0, 40)}"`)
      const glyph = (document.body.textContent ?? "").includes("✦")
      return { ai, glyph }
    })
    expect(found.ai, "elements drawn with the --ai tokens").toEqual([])
    expect(found.glyph, "a ✦ on screen").toBe(false)
  })
}
