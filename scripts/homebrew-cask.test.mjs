// node --test scripts/   (run by `pnpm test`)
import assert from "node:assert/strict"
import { test } from "node:test"

import { buildCask } from "./homebrew-cask.mjs"

const SHA = "a".repeat(64)

test("a stable version is the ranch cask, following the latest release", () => {
  const { file, content } = buildCask({ version: "0.4.0", sha256: SHA })
  assert.equal(file, "ranch.rb")
  assert.match(content, /^cask "ranch" do$/m)
  assert.match(content, /version "0\.4\.0"/)
  assert.match(content, new RegExp(`sha256 "${SHA}"`))
  assert.match(content, /strategy :github_latest/)
  assert.match(content, /conflicts_with cask: "ranch@beta"/)
  assert.match(content, /Ranch_#\{version\}_universal\.dmg/)
})

test("a pre-release is the ranch@beta cask, following every release", () => {
  const { file, content } = buildCask({ version: "0.4.0-rc.1", sha256: SHA })
  assert.equal(file, "ranch@beta.rb")
  assert.match(content, /^cask "ranch@beta" do$/m)
  assert.match(content, /version "0\.4\.0-rc\.1"/)
  assert.match(content, /strategy :github_releases/)
  assert.match(content, /conflicts_with cask: "ranch"$/m)
})

test("a version or a checksum that is not one is refused", () => {
  assert.throws(() => buildCask({ version: "latest", sha256: SHA }))
  assert.throws(() => buildCask({ version: "0.4.0", sha256: "abc" }))
})
