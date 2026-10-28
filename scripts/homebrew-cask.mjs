#!/usr/bin/env node
// Release pipeline: write the Homebrew cask of a version into the tap.
//
//   node scripts/homebrew-cask.mjs v0.4.0 --sha256 <dmg sha256> --tap ./homebrew-tap
//
// A stable version goes to `Casks/ranch.rb`, a pre-release to
// `Casks/ranch@beta.rb`: `brew install --cask bramvanosta/tap/ranch` follows
// stable releases only. Both install the universal DMG of the GitHub release.
import { mkdirSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

const REPO = "Bramvanosta/Ranch"

/** The cask's file name and content for one version. */
export function buildCask({ version, sha256 }) {
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error(`"${version}" is not a version`)
  }
  if (!/^[0-9a-f]{64}$/.test(sha256)) {
    throw new Error(`"${sha256}" is not a sha256`)
  }
  const prerelease = version.includes("-")
  const token = prerelease ? "ranch@beta" : "ranch"
  const other = prerelease ? "ranch" : "ranch@beta"
  // A pre-release is found among every release; a stable one is the latest.
  const livecheck = prerelease
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
  const { file, content } = buildCask({ version: tag.replace(/^v/, ""), sha256 })
  mkdirSync(join(tap, "Casks"), { recursive: true })
  writeFileSync(join(tap, "Casks", file), content)
  console.log(`Casks/${file}`)
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main(process.argv.slice(2))
