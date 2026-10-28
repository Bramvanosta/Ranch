import { describe, expect, it } from "vitest"

import pkg from "../../package.json"

describe("__APP_VERSION__", () => {
  // The constant comes from package.json, which the release pipeline
  // rewrites from the git tag, so a build shows the version it was made as.
  it("is package.json's version", () => {
    expect(__APP_VERSION__).toBe(pkg.version)
    expect(__APP_VERSION__).toMatch(/^\d+\.\d+\.\d+/)
  })
})
