#!/usr/bin/env node
// Release pipeline: write the Homebrew cask of a version into the tap.
//
//   node scripts/homebrew-cask.mjs v0.4.0 --sha256 <dmg sha256> --tap ./homebrew-tap
//
// `Casks/ranch.rb` follows stable versions only; `Casks/ranch@beta.rb`
// follows every version, release candidates and stable ones alike, so a beta
// install never stays behind the stable release. Both install the universal
// DMG of the GitHub release. A cask is never moved back to an older version
// (a re-run of an old tag leaves it alone).
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

import { compareVersions } from "./publish-update.mjs"

const REPO = "Bramvanosta/Ranch"

/** One cask's file name and content: `ranch@beta` when `beta`, else `ranch`. */
export function buildCask({ version, sha256, beta = version.includes("-") }) {
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error(`"${version}" is not a version`)
  }
  if (!/^[0-9a-f]{64}$/.test(sha256)) {
    throw new Error(`"${sha256}" is not a sha256`)
  }
  if (version.includes("-") && !beta) {
    throw new Error(`${version} is a pre-release: only ranch@beta follows it`)
  }
  const token = beta ? "ranch@beta" : "ranch"
  const other = beta ? "ranch" : "ranch@beta"
  // The beta is found among every release; the stable one is the latest.
  const livecheck = beta
    ? `  livecheck do
    url :url
    regex(/^v?(\\d+(?:\\.\\d+)+(?:-[0-9A-Za-z.-]+)?)$/i)
    strategy :github_releases
  end`
    : `  livecheck do
    url :url
    strategy :github_latest
  end`
  const content = `cask "${token}" do
  version "${version}"
  sha256 "${sha256}"

  url "https://github.com/${REPO}/releases/download/v#{version}/Ranch_#{version}_universal.dmg"
  name "Ranch"
  desc "Desktop client for BullMQ queues, with an MCP server for agents"
  homepage "https://github.com/${REPO}"

${livecheck}

  auto_updates true
  conflicts_with cask: "${other}"
  depends_on :macos

  app "Ranch.app"

  zap trash: [
    "~/Library/Application Support/dev.ranch.app",
    "~/Library/Caches/dev.ranch.app",
    "~/Library/Preferences/dev.ranch.app.plist",
    "~/Library/Saved Application State/dev.ranch.app.savedState",
    "~/Library/WebKit/dev.ranch.app",
  ]
end
`
  return { file: `${token}.rb`, content }
}

/** The casks a version updates: both for a stable version, the beta alone for a pre-release. */
export function buildCasks({ version, sha256 }) {
  const beta = buildCask({ version, sha256, beta: true })
  return version.includes("-") ? [beta] : [buildCask({ version, sha256, beta: false }), beta]
}

/** The version a cask's content names, or `null`. */
export function caskVersion(content) {
  return /^\s*version "([^"]+)"/m.exec(content ?? "")?.[1] ?? null
}

function main(argv) {
  const [tag, ...rest] = argv
  const flag = (name) => {
    const i = rest.indexOf(name)
    return i === -1 ? undefined : rest[i + 1]
  }
  const sha256 = flag("--sha256")
  const tap = flag("--tap")
  if (!tag || !sha256 || !tap) {
    console.error("usage: homebrew-cask.mjs <tag> --sha256 <hex> --tap <dir>")
    process.exit(1)
  }
  const version = tag.replace(/^v/, "")
  mkdirSync(join(tap, "Casks"), { recursive: true })
  for (const { file, content } of buildCasks({ version, sha256 })) {
    const path = join(tap, "Casks", file)
    const current = existsSync(path) ? caskVersion(readFileSync(path, "utf8")) : null
    if (current && compareVersions(version, current) < 0) {
      console.log(`Casks/${file} is at ${current}: left alone (this is ${version})`)
      continue
    }
    writeFileSync(path, content)
    console.log(`Casks/${file}`)
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main(process.argv.slice(2))
