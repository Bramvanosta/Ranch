// node --test scripts/   (run by `pnpm test`)
import assert from "node:assert/strict"
import { test } from "node:test"

import { buildCask, buildCasks, caskVersion } from "./homebrew-cask.mjs"

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

test("a stable version updates the beta too, so it never stays behind", () => {
  const casks = buildCasks({ version: "0.4.0", sha256: SHA })
  assert.deepEqual(
    casks.map((c) => c.file),
    ["ranch.rb", "ranch@beta.rb"],
  )
  const beta = casks[1].content
  assert.match(beta, /^cask "ranch@beta" do$/m)
  assert.match(beta, /version "0\.4\.0"/)
  assert.match(beta, /strategy :github_releases/)
})

test("a pre-release updates the beta alone", () => {
  const casks = buildCasks({ version: "0.5.0-rc.1", sha256: SHA })
  assert.deepEqual(
    casks.map((c) => c.file),
    ["ranch@beta.rb"],
  )
  assert.throws(() => buildCask({ version: "0.5.0-rc.1", sha256: SHA, beta: false }))
})

test("a cask's version is read back from its content", () => {
  assert.equal(caskVersion(buildCask({ version: "0.4.0", sha256: SHA }).content), "0.4.0")
  assert.equal(caskVersion('cask "ranch" do\nend\n'), null)
})
