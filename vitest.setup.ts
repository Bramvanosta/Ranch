import "@testing-library/jest-dom/vitest"

import { installRegistryShortcuts } from "@/commands/useRegistryShortcuts"

/**
 * Global test setup.
 *
 * jsdom doesn't ship the modern browser APIs that Radix UI primitives rely on
 * (ResizeObserver, hasPointerCapture). We polyfill the bare minimum here so
 * components that include Radix primitives (Dialog, RadioGroup, etc.) can
 * mount under test.
 */

if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof globalThis.ResizeObserver
}

// Radix uses pointer-capture APIs on its triggers; jsdom doesn't implement them.
if (typeof Element !== "undefined") {
  if (!Element.prototype.hasPointerCapture) {
    Element.prototype.hasPointerCapture = () => false
  }
  if (!Element.prototype.setPointerCapture) {
    Element.prototype.setPointerCapture = () => {}
  }
  if (!Element.prototype.releasePointerCapture) {
    Element.prototype.releasePointerCapture = () => {}
  }
  if (!Element.prototype.scrollIntoView) {
    Element.prototype.scrollIntoView = () => {}
  }
}

// The registry's window chords, installed once as the app's
// root (`GlobalCommands`) does, so components' command handlers answer keys.
installRegistryShortcuts()
