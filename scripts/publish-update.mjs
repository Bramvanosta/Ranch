#!/usr/bin/env node
// Release pipelines: publish one platform's update on GitHub, then recompose
// the updater manifests from what is there.
//
//   node scripts/publish-update.mjs v0.2.0 \
//     --platforms darwin-aarch64,darwin-x86_64 \
//     --artifact path/to/Ranch.app.tar.gz --signature path/to/Ranch.app.tar.gz.sig
//
// Layout (docs/releasing.md, "Auto-updates"):
//
//   releases/download/<tag>/<artifact>                 the signed archive the updater downloads
//   releases/download/<tag>/platform-<platform>.json   { signature, url } — one per platform
//   releases/download/update-feed/rc.json              manifest of the newest tag, stable or not
//   releases/download/update-feed/latest.json          manifest of the newest STABLE tag
//
// The channels sit on one fixed release, `update-feed`, because GitHub's
// `releases/latest/download/…` skips pre-releases and the app has a
// pre-release channel. That release is marked pre-release itself, so GitHub
// never shows it as the latest one.
//
// Every build job runs this for its own platform; none waits for the others.
// The manifest is therefore never edited: it is rebuilt from the platform files
// of the tag, so the job that finishes last publishes the complete one, and a
// re-run heals a manifest that lost a race. A manifest is never replaced by an
// older version (a re-run of an old tag leaves the channels alone).
//
// Environment: GITHUB_TOKEN (contents: write), GITHUB_REPOSITORY (owner/name);
// GITHUB_API_URL and GITHUB_SERVER_URL when not github.com (Actions sets them).
import { randomUUID } from "node:crypto"
import { existsSync, readFileSync } from "node:fs"
import { basename } from "node:path"

/** The fixed release whose assets are the channel manifests. */
export const FEED_TAG = "update-feed"
/** Every key a Tauri updater may ask for; the manifest lists those that exist for the tag. */
export const KNOWN_PLATFORMS = [
  "darwin-aarch64",
  "darwin-x86_64",
  "linux-x86_64",
  "linux-aarch64",
  "windows-x86_64",
]

export function parseVersion(text) {
  const m = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/.exec(text ?? "")
  if (!m) return null
  return { core: [Number(m[1]), Number(m[2]), Number(m[3])], pre: m[4] ? m[4].split(".") : [] }
}

/** Semver precedence: negative when a < b. A pre-release sorts before its release. */
export function compareVersions(a, b) {
  const x = parseVersion(a)
  const y = parseVersion(b)
  if (!x || !y) throw new Error(`cannot compare "${a}" and "${b}"`)
  for (let i = 0; i < 3; i++) if (x.core[i] !== y.core[i]) return x.core[i] - y.core[i]
  if (x.pre.length === 0 || y.pre.length === 0) return y.pre.length - x.pre.length
  for (let i = 0; i < Math.max(x.pre.length, y.pre.length); i++) {
    const p = x.pre[i]
    const q = y.pre[i]
    if (p === undefined) return -1
    if (q === undefined) return 1
    const pn = /^\d+$/.test(p)
    const qn = /^\d+$/.test(q)
    if (pn && qn && Number(p) !== Number(q)) return Number(p) - Number(q)
    if (pn !== qn) return pn ? -1 : 1
    if (p !== q) return p < q ? -1 : 1
  }
  return 0
}

/** Markdown the dialog would show raw: keep the words, drop the markers. */
const plain = (text) => text.replace(/\*\*(.+?)\*\*/g, "$1").replace(/`([^`]+)`/g, "$1")

/**
 * What the update dialog shows — written for the person about to click
 * *Install*, not for the changelog's reader:
 *
 *   - the **summary paragraph** of the version's section of CHANGELOG.md
 *     (`## [v0.2.0] — …`): whatever stands between its heading and its first
 *     `###`. Write one for every release;
 *   - without one, the lead of each bullet (its bold sentence), a few of them.
 *
 * Plain text: the dialog does not render Markdown. A pre-release has no
 * section of its own: `null`. (release.yml puts the same text on the release page.)
 */
export function extractNotes(changelog, version, maxItems = 8) {
  const lines = (changelog ?? "").split("\n")
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
  const start = lines.findIndex((l) => new RegExp(`^## \\[v?${escaped}\\]`).test(l))
  if (start === -1) return null
  const rest = lines.slice(start + 1)
  const end = rest.findIndex((l) => /^## \[/.test(l))
  const body = end === -1 ? rest : rest.slice(0, end)

  const firstHeading = body.findIndex((l) => /^### /.test(l))
  const summary = (firstHeading === -1 ? body : body.slice(0, firstHeading)).join("\n").trim()
  if (summary && !summary.startsWith("- ")) return plain(summary)

  const leads = body
    .filter((l) => /^- /.test(l))
    .map((l) => /^- \*\*(.+?)\*\*/.exec(l)?.[1] ?? l.slice(2).split(/(?<=[.:;])\s/)[0])
    .map((lead) => `• ${plain(lead).replace(/[.:;]$/, "")}`)
  if (leads.length === 0) return null
  const more = leads.length > maxItems ? `\n… and ${leads.length - maxItems} more` : ""
  return leads.slice(0, maxItems).join("\n") + more
}

/** The static JSON a Tauri v2 updater reads. */
export function buildManifest({ version, notes, pubDate, platforms }) {
  return { version, notes, pub_date: pubDate, platforms }
}

/** One asset per platform on the version's release: several jobs, no shared file. */
export const platformAssetName = (platform) => `platform-${platform}.json`

/** The public download URL of an asset: no API, no token — what an installed app can fetch. */
export function assetDownloadUrl(repository, tag, name, server = "https://github.com") {
  const root = `${server.replace(/\/$/, "")}/${repository}/releases/download`
  return `${root}/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`
}

/** rc.json follows every tag; latest.json only stable ones. */
export const channelsFor = (version) =>
  version.includes("-") ? ["rc.json"] : ["rc.json", "latest.json"]

/** A channel moves to `version` unless it serves a newer one. The same version is rewritten: a re-run heals. */
export function supersedes(version, current) {
  return !parseVersion(current) || compareVersions(version, current) >= 0
}

/** A release's `upload_url` is a URI template (`…/assets{?name,label}`). */
export function uploadUrl(template, name) {
  return `${template.replace(/\{[^}]*\}$/, "")}?name=${encodeURIComponent(name)}`
}

export function parseArgs(argv) {
  const [tag, ...rest] = argv
  const out = { tag, platforms: [], artifact: null, signature: null, notes: null }
  for (let i = 0; i < rest.length; i++) {
    const value = rest[i + 1]
    if (rest[i] === "--platforms") out.platforms = (value ?? "").split(",").filter(Boolean)
    else if (rest[i] === "--artifact") out.artifact = value
    else if (rest[i] === "--signature") out.signature = value
    else if (rest[i] === "--notes") out.notes = value
    else continue
    i++
  }
  return out
}

/**
 * The few GitHub REST calls the publication needs. `publishUpdate` only talks
 * to this object, so the tests hand it an in-memory GitHub instead.
 */
export function createGitHub({
  token,
  repository,
  api = "https://api.github.com",
  fetch = globalThis.fetch,
}) {
  const base = `${api.replace(/\/$/, "")}/repos/${repository}`
  const auth = { authorization: `Bearer ${token}`, "x-github-api-version": "2022-11-28" }

  async function call(method, url, { json, body, type, allow = [] } = {}) {
    const response = await fetch(url.startsWith("/") ? `${base}${url}` : url, {
      method,
      headers: {
        ...auth,
        accept: "application/vnd.github+json",
        ...(json ? { "content-type": "application/json" } : type ? { "content-type": type } : {}),
      },
      body: json ? JSON.stringify(json) : body,
    })
    if (allow.includes(response.status)) return null
    if (!response.ok) {
      const path = url.startsWith("/") ? url : new URL(url).pathname
      const detail = (await response.text()).slice(0, 300)
      throw new Error(`${method} ${path} → ${response.status} ${detail}`)
    }
    return response.status === 204 ? null : response.json()
  }

  return {
    releaseByTag: (tag) =>
      call("GET", `/releases/tags/${encodeURIComponent(tag)}`, { allow: [404] }),
    // 422: a release for that tag already exists (another job got there first).
    createRelease: (fields) => call("POST", "/releases", { json: fields, allow: [422] }),
    async assets(release) {
      const all = []
      for (let page = 1; ; page++) {
        const batch = await call("GET", `/releases/${release.id}/assets?per_page=100&page=${page}`)
        all.push(...batch)
        if (batch.length < 100) return all
      }
    },
    // The API answers with a redirect to a signed storage URL, which must be
    // fetched without our token (it carries its own authorization). Unlike the
    // public download URL, this works on a private repository too.
    async download(asset) {
      const response = await fetch(`${base}/releases/assets/${asset.id}`, {
        headers: { ...auth, accept: "application/octet-stream" },
        redirect: "manual",
      })
      const location = response.headers.get("location")
      const redirected = response.status >= 300 && response.status < 400 && location
      const final = redirected ? await fetch(location) : response
      if (!final.ok) throw new Error(`download ${asset.name} → ${final.status}`)
      return final.text()
    },
    upload: (release, name, body, type) =>
      call("POST", uploadUrl(release.upload_url, name), { body, type }),
    remove: (asset) => call("DELETE", `/releases/assets/${asset.id}`, { allow: [404] }),
    // 422: the name is taken (a concurrent publication renamed its own first).
    rename: (asset, name) =>
      call("PATCH", `/releases/assets/${asset.id}`, { json: { name }, allow: [422] }),
  }
}

/**
 * Replace an asset without a moment where it is missing: upload under a
 * temporary name, then drop the old one and take its name. GitHub assets
 * cannot be overwritten in place, and a delete-then-upload would answer 404
 * to an updater checking at that moment.
 */
async function replaceAsset(gh, release, name, body, type) {
  const fresh = await gh.upload(release, `${name}.${randomUUID().slice(0, 8)}.tmp`, body, type)
  for (let attempt = 0; attempt < 3; attempt++) {
    for (const old of (await gh.assets(release)).filter((a) => a.name === name))
      await gh.remove(old)
    if (await gh.rename(fresh, name)) return
  }
  await gh.remove(fresh)
  throw new Error(`could not replace ${name} on ${release.tag_name}`)
}

async function readJson(gh, asset, log) {
  try {
    return JSON.parse(await gh.download(asset))
  } catch (error) {
    log(`  ${asset.name} is unreadable (${error.message}): treated as absent`)
    return null
  }
}

async function feedRelease(gh) {
  const existing = await gh.releaseByTag(FEED_TAG)
  if (existing) return existing
  const created = await gh.createRelease({
    tag_name: FEED_TAG,
    name: "Update feed",
    body: "What the desktop app reads to find updates: `latest.json` (stable) and `rc.json` (pre-releases), replaced at every release. Nothing to download here: see the releases.",
    draft: false,
    prerelease: true,
    make_latest: "false",
  })
  return created ?? (await gh.releaseByTag(FEED_TAG))
}

/**
 * The whole publication of one job. `artifact` is `{ name, read }`: the job
 * has normally attached the archive already (`gh release upload`); it is only
 * uploaded here when it has not.
 */
export async function publishUpdate({
  gh,
  repository,
  server,
  tag,
  platforms,
  artifact,
  signature,
  notes,
  now = new Date(),
  log = console.log,
  wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
}) {
  const version = tag.replace(/^v/, "")

  // The release is created by another job; give it a moment to show.
  let release = null
  for (let attempt = 0; attempt < 10 && !release; attempt++) {
    release = await gh.releaseByTag(tag)
    if (!release) await wait(3_000)
  }
  if (!release) throw new Error(`no release for ${tag}`)

  // 1. The archive, then what points at it.
  log(`Ranch ${version}: ${platforms.join(", ")}`)
  let archive = (await gh.assets(release)).find((a) => a.name === artifact.name)
  if (!archive) {
    const bytes = await artifact.read()
    archive = await gh.upload(release, artifact.name, bytes, "application/octet-stream")
    log(`  uploaded ${archive.name}`)
  }
  // The asset's own name: GitHub rewrites some characters on upload.
  const entry = { signature, url: assetDownloadUrl(repository, tag, archive.name, server) }
  const entryJson = JSON.stringify(entry, null, 2)
  for (const p of platforms) {
    await replaceAsset(gh, release, platformAssetName(p), entryJson, "application/json")
    log(`  ${tag}/${platformAssetName(p)}`)
  }

  // 2. The manifest of this tag, from every platform published so far.
  const assets = await gh.assets(release)
  const found = {}
  for (const p of KNOWN_PLATFORMS) {
    const asset = assets.find((a) => a.name === platformAssetName(p))
    const value = asset && (await readJson(gh, asset, log))
    if (value) found[p] = value
  }
  const manifest = buildManifest({ version, notes, pubDate: now.toISOString(), platforms: found })
  const body = JSON.stringify(manifest, null, 2)
  log(`manifest: ${Object.keys(found).join(", ")}`)

  // 3. The channels.
  const feed = await feedRelease(gh)
  const feedAssets = await gh.assets(feed)
  for (const channel of channelsFor(version)) {
    const asset = feedAssets.find((a) => a.name === channel)
    const current = asset ? await readJson(gh, asset, log) : null
    if (!supersedes(version, current?.version)) {
      log(`  ${channel} is at ${current.version}: left alone (this is ${version})`)
      continue
    }
    await replaceAsset(gh, feed, channel, body, "application/json")
    log(`  ${FEED_TAG}/${channel} → ${version}`)
  }
  return manifest
}

async function main() {
  const args = parseArgs(process.argv.slice(2))
  const version = args.tag?.replace(/^v/, "")
  const { GITHUB_TOKEN, GITHUB_REPOSITORY, GITHUB_API_URL, GITHUB_SERVER_URL } = process.env
  const fail = (message) => {
    console.error(`publish-update: ${message}`)
    process.exit(1)
  }
  if (!parseVersion(version)) fail(`"${args.tag}" is not a version tag`)
  if (!GITHUB_TOKEN) fail("GITHUB_TOKEN is not set")
  if (!GITHUB_REPOSITORY) fail("GITHUB_REPOSITORY is not set (owner/name)")
  if (!args.artifact || !args.signature || args.platforms.length === 0)
    fail("--platforms, --artifact and --signature are required")
  for (const p of args.platforms)
    if (!KNOWN_PLATFORMS.includes(p))
      fail(`unknown platform "${p}" (known: ${KNOWN_PLATFORMS.join(", ")})`)
  const signature = readFileSync(args.signature, "utf8").trim()
  if (!signature) fail(`${args.signature} is empty`)

  try {
    await publishUpdate({
      gh: createGitHub({ token: GITHUB_TOKEN, repository: GITHUB_REPOSITORY, api: GITHUB_API_URL }),
      repository: GITHUB_REPOSITORY,
      server: GITHUB_SERVER_URL,
      tag: args.tag,
      platforms: args.platforms,
      artifact: { name: basename(args.artifact), read: () => readFileSync(args.artifact) },
      signature,
      notes: args.notes
        ? readFileSync(args.notes, "utf8").trim()
        : (existsSync("CHANGELOG.md") &&
            extractNotes(readFileSync("CHANGELOG.md", "utf8"), version)) ||
          `Ranch ${version}`,
    })
  } catch (error) {
    fail(error.message)
  }
}

if (import.meta.url === `file://${process.argv[1]}`) await main()
