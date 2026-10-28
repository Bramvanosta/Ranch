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
      await open(page, "/connections/c-local/queues/bull%3Aemails?view=jobs&state=waiting")
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
      await open(page, "/connections/c-prod/queues/bull%3Aemails/1232")
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
  {
    name: "palette, a job found",
    run: async (page) => {
      await open(page, "/")
      await page.keyboard.press("ControlOrMeta+k")
      // Type once the palette holds the focus, not while it opens.
      await page.getByTestId("command-palette-input").fill("#1211")
      await page.getByText("send-reminder #1211").waitFor()
    },
  },
  {
    name: "agent approval",
    run: async (page) => {
      await open(page, "/connections/c-local")
      await page.evaluate(() =>
        (
          window as unknown as { ranchPreview: { requestApproval(): unknown } }
        ).ranchPreview.requestApproval(),
      )
      await page.getByRole("dialog").waitFor()
    },
  },
  {
    name: "drain on a protected connection",
    run: async (page) => {
      await open(page, "/connections/c-prod/queues/bull%3Aemails?view=jobs&state=waiting")
      await page.getByTestId("queue-maintenance-trigger").click()
      await page.getByTestId("queue-maintenance-drain").click()
      await page.getByRole("dialog").waitFor()
    },
  },
  {
    name: "maintenance menu",
    run: async (page) => {
      await open(page, "/connections/c-local/queues/bull%3Aemails?view=jobs&state=failed")
      await page.getByTestId("queue-maintenance-trigger").click()
      await page.getByTestId("queue-maintenance-drain").waitFor()
    },
  },
  {
    name: "connection menu",
    run: async (page) => {
      await open(page, "/")
      await page
        .getByRole("complementary", { name: "Sidebar" })
        .getByText("playground")
        .click({ button: "right" })
      await page.getByRole("menu").waitFor()
    },
  },
  {
    name: "bulk selection",
    run: async (page) => {
      await open(page, "/connections/c-local/queues/bull%3Aemails?view=jobs&state=failed")
      const boxes = page.getByRole("checkbox")
      await boxes.nth(1).click()
      await boxes.nth(2).click()
      await page.getByTestId("bulk-action-bar").waitFor()
    },
  },
  {
    name: "job inspector, Ask AI",
    run: async (page) => {
      await open(page, "/connections/c-local/queues/bull%3Aemails?view=jobs&state=failed&job=192")
      await page
        .getByRole("button", { name: /Ask AI/ })
        .last()
        .click()
      await page.getByRole("menu").waitFor()
    },
  },
  {
    name: "first launch, the wizard",
    run: async (page) => {
      await open(page, "/?preview=empty")
      await page.getByRole("button", { name: "Add a connection" }).click()
      await page.getByTestId("connection-sheet").waitFor()
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
