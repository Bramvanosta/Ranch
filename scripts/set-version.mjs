#!/usr/bin/env node
// Release pipelines: write one version everywhere it lives, from the git tag.
//
//   node scripts/set-version.mjs v0.2.0        → 0.2.0
//   node scripts/set-version.mjs v0.2.0-rc.1   → 0.2.0-rc.1
//
// package.json, src-tauri/tauri.conf.json, src-tauri/Cargo.toml — and the
// `ranch` entry of Cargo.lock, so that `cargo build --locked` still holds.
// Prints the version, and whether it is a pre-release (semver: it has a `-`).
//
// A pre-release also follows the pre-release update channel: its updater reads
// `…/update-feed/rc.json` (every tag) instead of `…/update-feed/latest.json`
// (stable tags only) — so an RC is offered the next RC, and a stable build
// never is. Both are assets of the `update-feed` GitHub release
// (scripts/publish-update.mjs).
import { readFileSync, writeFileSync } from "node:fs"

const raw = process.argv[2]
const version = raw?.replace(/^v/, "")
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`set-version: "${raw}" is not a version (expected vX.Y.Z or vX.Y.Z-rc.N)`)
  process.exit(1)
}

function edit(path, replace) {
  const before = readFileSync(path, "utf8")
  const after = replace(before)
  if (after === before && !before.includes(version)) {
    console.error(`set-version: nothing to change in ${path} — has its layout changed?`)
    process.exit(1)
  }
  writeFileSync(path, after)
}

const json = (path) =>
  edit(path, (text) => text.replace(/("version":\s*")[^"]+(")/, `$1${version}$2`))

json("package.json")
json("src-tauri/tauri.conf.json")
// The first `version =` of Cargo.toml is the package's.
edit("src-tauri/Cargo.toml", (text) =>
  text.replace(/^version = "[^"]+"/m, `version = "${version}"`),
)
edit("src-tauri/Cargo.lock", (text) =>
  text.replace(/(\[\[package\]\]\nname = "ranch"\nversion = ")[^"]+(")/, `$1${version}$2`),
)

const STABLE_FEED = "/releases/download/update-feed/latest.json"
const prerelease = version.includes("-")
if (prerelease) {
  edit("src-tauri/tauri.conf.json", (text) => {
    if (!text.includes(STABLE_FEED)) {
      console.error(`set-version: no \`${STABLE_FEED}\` endpoint in tauri.conf.json to switch`)
      process.exit(1)
    }
    return text.replace(STABLE_FEED, "/releases/download/update-feed/rc.json")
  })
}
console.log(`version=${version}`)
console.log(`prerelease=${prerelease}`)
