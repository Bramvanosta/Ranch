// First: a window opened at a route sets it before the router starts.
import "./lib/windows/openedRoute"
import { getCurrentWindow } from "@tauri-apps/api/window"
import React, { useEffect } from "react"
import ReactDOM from "react-dom/client"
import "./styles/globals.css"
import { App } from "./App"
import { installHttpBackend, loadServerInfo } from "./lib/runtime/httpBackend"
import { setRuntime } from "./lib/runtime/runtime"
import { isTauri } from "./lib/runtime/tauriShim"

function AppWrapper() {
  useEffect(() => {
    // Show window after React is ready (template pattern: window starts hidden
    // per tauri.conf.json `visible: false` to avoid white-flash on launch).
    getCurrentWindow().show()
  }, [])

  return <App />
}

async function boot() {
  // One bundle, three places to run (see `lib/runtime/runtime.ts`):
  if (isTauri()) {
    setRuntime("desktop")
  } else if (import.meta.env.DEV && import.meta.env.VITE_RANCH_BACKEND !== "http") {
    // "Ranch web": `pnpm dev` opened in a plain browser; a stateful in-memory
    // stand-in makes every screen reviewable. Dev only, and a dynamic import:
    // it is not part of production bundles.
    const { installFakeBackend } = await import("./backend/fake")
    installFakeBackend()
    setRuntime("preview")
  } else {
    // Served by `ranch-server` (the Docker image): the same
    // commands, over HTTP. (`VITE_RANCH_BACKEND=http pnpm dev` does the same
    // against a local server, through Vite's proxy.)
    installHttpBackend()
    setRuntime("server")
    await loadServerInfo()
  }
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <AppWrapper />
    </React.StrictMode>,
  )
}

void boot()
