import path from "node:path"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vitest/config"

import pkg from "./package.json" with { type: "json" }

/**
 * Vitest config: jsdom, Testing Library, the `@` alias.
 *
 * Uses jsdom for DOM emulation (Testing Library). The setup file installs
 * `@testing-library/jest-dom` matchers and stubs the Tauri `invoke` bridge
 * so component tests can run without the Rust backend.
 */
export default defineConfig({
  plugins: [react()],
  define: { __APP_VERSION__: JSON.stringify(pkg.version) },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./vitest.setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    css: false,
  },
})
