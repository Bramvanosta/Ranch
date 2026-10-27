import { expect, type Page, test } from "@playwright/test"

import { detectContrast } from "../contrast/detect"
import { detectOverflow } from "../overflow/detect"
import { SIZES, sizeLabel, THEMES } from "../overflow/routes"

// The sheets (and the palette), in both themes and both sizes:
// no text that does not fit, no text under WCAG AA.
const SETTLE_MS = 500

async function open(page: Page, path: string) {
  const url = new URL(path, "http://preview.local")
  if (!url.searchParams.has("preview")) url.searchParams.set("preview", "full")
  await page.goto(`${url.pathname}${url.search}`)
  await page.waitForFunction(() => (document.body.innerText || "").trim().length > 0)
  await page.evaluate(() => document.fonts.ready)
  await page.waitForTimeout(SETTLE_MS)
}

async function check(page: Page) {
  await page.waitForTimeout(SETTLE_MS)
  const overflow = await page.evaluate(detectOverflow)
  expect(
    overflow.map((v) => `${v.kind} ${v.px}px "${v.text}" (${v.selector})`),
    "text that does not fit",
  ).toEqual([])
  const contrast = await page.evaluate(detectContrast)
  expect(
    contrast.map((v) => `${v.ratio}:1 < ${v.needed} "${v.text}" (${v.selector})`),
    "text under WCAG AA",
  ).toEqual([])
}

const SHEETS: Array<{ name: string; run: (page: Page) => Promise<void> }> = [
  {
    name: "new connection, step 1",
    run: async (page) => {
      await open(page, "/")
      await page.getByTestId("sidebar-new-connection").click()
      await page.getByTestId("connection-sheet").waitFor()
    },
  },
  {
    name: "new connection, step 2",
    run: async (page) => {
      await open(page, "/")
      await page.getByTestId("sidebar-new-connection").click()
      await page.getByLabel("Host").fill("redis.example.com")
      await page.keyboard.press("Enter")
      await page.getByTestId("probe-facts").waitFor()
    },
  },
  {
    name: "add job",
    run: async (page) => {
      await open(page, "/connections/c-local/queues/emails?view=jobs&state=waiting")
      await page
        .getByRole("button", { name: /^Add job/ })
        .first()
        .click()
      await page.getByRole("dialog").waitFor()
    },
  },
  {
    name: "remove on a protected connection",
    run: async (page) => {
      await open(page, "/connections/c-prod/queues/emails/1232")
      await page.getByTestId("job-actions-remove").click()
      await page.getByRole("dialog").waitFor()
    },
  },
  {
    name: "palette",
    run: async (page) => {
      await open(page, "/")
      await page.keyboard.press("ControlOrMeta+k")
      await page.getByTestId("command-palette").waitFor()
    },
  },
]

for (const size of SIZES) {
  for (const theme of THEMES) {
    test.describe(`${theme} ${sizeLabel(size)}`, () => {
      test.use({ colorScheme: theme, viewport: size })
      for (const sheet of SHEETS) {
        test(sheet.name, async ({ page }) => {
          await sheet.run(page)
          await check(page)
        })
      }
    })
  }
}
