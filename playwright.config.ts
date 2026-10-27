import { defineConfig, devices } from "@playwright/test"

// The overflow check (NFR42, `pnpm check:overflow`): every screen of the web
// preview, in both themes and two window sizes, must fit its text.
// Chromium only: it catches layout overflow as well as the macOS webview would,
// and it is the browser the CI image carries.
export default defineConfig({
  testDir: "e2e",
  // One Vite server, a deterministic order.
  fullyParallel: false,
  workers: 1,
  // A cold Vite compile can time out the first navigation on CI; nothing more.
  retries: process.env.CI ? 1 : 0,
  forbidOnly: !!process.env.CI,
  reporter: "list",
  use: {
    ...devices["Desktop Chrome"],
    baseURL: "http://localhost:1420",
    reducedMotion: "reduce",
  },
  projects: [
    { name: "detector", testMatch: "overflow/detect.spec.ts" },
    { name: "screens", testMatch: "overflow/screens.spec.ts" },
    // Runs after the sweep: writes the report and rejects stale allow-list entries.
    { name: "allowlist", testMatch: "overflow/allowlist.spec.ts", dependencies: ["screens"] },
    // With AI features off, nothing of AI on any screen.
    { name: "ai-off", testMatch: "ai/ai-off.spec.ts" },
    // Text contrast, every screen, both themes.
    { name: "contrast", testMatch: "contrast/contrast.spec.ts" },
    // The sheets and the palette, overflow and contrast.
    { name: "sheets", testMatch: "sheets/sheets.spec.ts" },
  ],
  webServer: {
    command: "pnpm dev",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
})
