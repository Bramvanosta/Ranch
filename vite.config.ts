import path from "node:path"
import tailwindcss from "@tailwindcss/vite"
import { tanstackRouter } from "@tanstack/router-plugin/vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"

import pkg from "./package.json" with { type: "json" }

const host = process.env.TAURI_DEV_HOST
// Work on the server image's UI with hot reload:
//   RANCH_SERVER_URL=http://127.0.0.1:8080 VITE_RANCH_BACKEND=http pnpm dev
// `/api` is proxied to a running `ranch-server`.
const ranchServer: string | undefined = process.env.RANCH_SERVER_URL

// https://vite.dev/config/
export default defineConfig(async () => ({
  // The app's own version, from package.json — which the release pipeline
  // rewrites from the git tag (`scripts/set-version.mjs`) before building.
  define: { __APP_VERSION__: JSON.stringify(pkg.version) },
  // tanstackRouter() MUST precede react() in the plugins array
  // — the router plugin transforms route files at compile time and the
  // react plugin's HMR transform must run after.
  plugins: [tanstackRouter({ target: "react", autoCodeSplitting: true }), react(), tailwindcss()],

  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
    proxy: ranchServer
      ? { "/api": { target: ranchServer, changeOrigin: false }, "/healthz": ranchServer }
      : undefined,
  },
}))
