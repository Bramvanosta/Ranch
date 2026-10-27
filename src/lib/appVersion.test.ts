import { describe, expect, it } from "vitest"

import pkg from "../../package.json"

describe("__APP_VERSION__", () => {
  // The status bar used to say v0.1.0 whatever the build. The constant comes
  // from package.json, which the release pipeline rewrites from the git tag.
  it("is package.json's version", () => {
    expect(__APP_VERSION__).toBe(pkg.version)
    expect(__APP_VERSION__).toMatch(/^\d+\.\d+\.\d+/)
  })
})
