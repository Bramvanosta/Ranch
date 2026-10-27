// node --test scripts/   (run by `pnpm test`)
import assert from "node:assert/strict"
import { test } from "node:test"

import {
  assetDownloadUrl,
  buildManifest,
  channelsFor,
  compareVersions,
  createGitHub,
  extractNotes,
  parseArgs,
  parseVersion,
  publishUpdate,
  supersedes,
  uploadUrl,
} from "./publish-update.mjs"

test("versions parse with or without the v, and garbage does not", () => {
  assert.deepEqual(parseVersion("v0.2.0"), { core: [0, 2, 0], pre: [] })
  assert.deepEqual(parseVersion("0.2.0-rc.10"), { core: [0, 2, 0], pre: ["rc", "10"] })
  assert.equal(parseVersion("latest"), null)
  assert.equal(parseVersion(undefined), null)
})

test("semver precedence: a release is newer than its own release candidates", () => {
  const sorted = ["0.2.0", "0.2.0-rc.10", "0.10.0", "0.2.0-rc.2", "0.2.1-rc.1", "0.2.0-rc.9"].sort(
    compareVersions,
  )
  assert.deepEqual(sorted, [
    "0.2.0-rc.2",
    "0.2.0-rc.9",
    "0.2.0-rc.10",
    "0.2.0",
    "0.2.1-rc.1",
    "0.10.0",
  ])
  assert.equal(compareVersions("v0.2.0", "0.2.0"), 0)
  assert.ok(compareVersions("0.2.0-rc.1", "0.2.0-rc.1.1") < 0)
  assert.ok(compareVersions("0.2.0-1", "0.2.0-rc") < 0) // numeric identifiers sort first
  assert.throws(() => compareVersions("0.2.0", "soon"))
})

test("the manifest is the static JSON a Tauri v2 updater reads", () => {
  const entry = {
    signature: "c2ln",
    url: "https://github.com/owner/ranch/releases/download/v0.2.0/Ranch.app.tar.gz",
  }
  assert.deepEqual(
    buildManifest({
      version: "0.2.0",
      notes: "Ranch 0.2.0",
      pubDate: "2026-09-21T16:00:00.000Z",
      platforms: { "darwin-aarch64": entry, "darwin-x86_64": entry },
    }),
    {
      version: "0.2.0",
      notes: "Ranch 0.2.0",
      pub_date: "2026-09-21T16:00:00.000Z",
      platforms: { "darwin-aarch64": entry, "darwin-x86_64": entry },
    },
  )
})

test("release notes: the summary paragraph of the version's section, as plain text", () => {
  const changelog = [
    "# Changelog",
    "",
    "## [Unreleased]",
    "- not yet",
    "",
    "## [v0.2.0] — 2026-09-22",
    "",
    "The first real release: **groups**, agents behind an approval, and an app that `updates itself`.",
    "",
    "### Fixed",
    "- **Waiting jobs were missing from the lists.** BullMQ writes `atm` only when an attempt ends.",
    "",
    "## [v0.1.0] — 2026-05-03 — Scaffold",
    "- first",
  ].join("\n")
  assert.equal(
    extractNotes(changelog, "0.2.0"),
    "The first real release: groups, agents behind an approval, and an app that updates itself.",
  )
  // 0.2.0 must not match 0.2.0-rc.1's (absent) section, nor the dot match any character.
  assert.equal(extractNotes(changelog, "0.2.0-rc.1"), null)
  assert.equal(extractNotes(changelog, "0x2.0"), null)
})

test("release notes without a summary: the lead of each bullet, a few of them", () => {
  const bullets = Array.from(
    { length: 10 },
    (_, i) => `- **Thing ${i + 1} is fixed.** Long explanation with \`code\`.`,
  )
  const changelog = [
    "## [v0.3.0] — 2026-10-01",
    "",
    "### Fixed",
    ...bullets,
    "- a plain one: with details after",
    "",
  ].join("\n")
  const notes = extractNotes(changelog, "0.3.0").split("\n")
  assert.equal(notes[0], "• Thing 1 is fixed")
  assert.equal(notes.length, 9)
  assert.equal(notes.at(-1), "… and 3 more")
  assert.equal(
    extractNotes("## [v0.4.0]\n\n### Fixed\n- a plain one: with details after\n", "0.4.0"),
    "• a plain one",
  )
  assert.equal(extractNotes("## [v0.5.0]\n\n## [v0.4.0]\n- x", "0.5.0"), null)
})

test("arguments: one artifact may serve several platforms (a universal macOS build)", () => {
  assert.deepEqual(
    parseArgs([
      "v0.2.0",
      "--platforms",
      "darwin-aarch64,darwin-x86_64",
      "--artifact",
      "a.tar.gz",
      "--signature",
      "a.tar.gz.sig",
    ]),
    {
      tag: "v0.2.0",
      platforms: ["darwin-aarch64", "darwin-x86_64"],
      artifact: "a.tar.gz",
      signature: "a.tar.gz.sig",
      notes: null,
    },
  )
  assert.deepEqual(parseArgs(["v0.2.0"]).platforms, [])
})

test("download URLs, upload URLs and channels", () => {
  assert.equal(
    assetDownloadUrl("Bramvanosta/ranch", "v0.3.0-rc.1", "Ranch_0.3.0-rc.1_amd64.AppImage"),
    "https://github.com/Bramvanosta/ranch/releases/download/v0.3.0-rc.1/Ranch_0.3.0-rc.1_amd64.AppImage",
  )
  assert.equal(
    uploadUrl("https://uploads.github.com/repos/o/r/releases/7/assets{?name,label}", "rc.json"),
    "https://uploads.github.com/repos/o/r/releases/7/assets?name=rc.json",
  )
  assert.deepEqual(channelsFor("0.3.0-rc.1"), ["rc.json"])
  assert.deepEqual(channelsFor("0.3.0"), ["rc.json", "latest.json"])
  assert.ok(supersedes("0.3.0", undefined))
  assert.ok(supersedes("0.3.0", "0.3.0")) // a re-run rewrites its own manifest
  assert.ok(supersedes("0.3.0", "0.3.0-rc.4"))
  assert.ok(!supersedes("0.3.0-rc.2", "0.3.0"))
})

/** An in-memory GitHub with the surface `publishUpdate` uses. */
function fakeGitHub() {
  const releases = new Map()
  let nextId = 1
  const created = []
  const find = (asset) =>
    [...releases.values()].flatMap((r) => r.assets).find((a) => a.id === asset.id)
  const gh = {
    releases,
    created,
    addRelease(tag, fields = {}) {
      const id = nextId++
      const release = {
        id,
        tag_name: tag,
        upload_url: `https://uploads.example/releases/${id}/assets{?name,label}`,
        assets: [],
        ...fields,
      }
      releases.set(tag, release)
      return release
    },
    feed: (name) => {
      const asset = releases.get("update-feed")?.assets.find((a) => a.name === name)
      return asset && JSON.parse(asset.body)
    },
    async releaseByTag(tag) {
      return releases.get(tag) ?? null
    },
    async createRelease(fields) {
      if (releases.has(fields.tag_name)) return null
      created.push(fields)
      return gh.addRelease(fields.tag_name, fields)
    },
    async assets(release) {
      return releases.get(release.tag_name).assets.map(({ id, name }) => ({ id, name }))
    },
    async download(asset) {
      return find(asset).body
    },
    async upload(release, name, body) {
      const asset = { id: nextId++, name, body: String(body) }
      releases.get(release.tag_name).assets.push(asset)
      return { id: asset.id, name }
    },
    async remove(asset) {
      for (const r of releases.values()) r.assets = r.assets.filter((a) => a.id !== asset.id)
      return null
    },
    async rename(asset, name) {
      const owner = [...releases.values()].find((r) => r.assets.some((a) => a.id === asset.id))
      if (owner.assets.some((a) => a.name === name)) return null
      find(asset).name = name
      return { id: asset.id, name }
    },
  }
  return gh
}

const publish = (gh, tag, platforms, artifactName, overrides = {}) =>
  publishUpdate({
    gh,
    repository: "Bramvanosta/ranch",
    tag,
    platforms,
    artifact: { name: artifactName, read: () => `bytes of ${artifactName}` },
    signature: `sig of ${artifactName}`,
    notes: `Ranch ${tag}`,
    now: new Date("2026-10-27T12:00:00.000Z"),
    log: () => {},
    wait: async () => {},
    ...overrides,
  })

const names = (gh, tag) =>
  gh.releases
    .get(tag)
    .assets.map((a) => a.name)
    .sort()

test("each job publishes its platform; the last one completes the manifest", async () => {
  const gh = fakeGitHub()
  const release = gh.addRelease("v0.3.0")
  // The job attached its AppImage before publishing: it is not uploaded twice.
  await gh.upload(release, "Ranch_0.3.0_amd64.AppImage", "built")

  await publish(gh, "v0.3.0", ["linux-x86_64"], "Ranch_0.3.0_amd64.AppImage")
  assert.deepEqual(Object.keys(gh.feed("latest.json").platforms), ["linux-x86_64"])
  assert.deepEqual(gh.feed("rc.json"), gh.feed("latest.json"))

  await publish(gh, "v0.3.0", ["darwin-aarch64", "darwin-x86_64"], "Ranch.app.tar.gz")
  const latest = gh.feed("latest.json")
  assert.deepEqual(latest, {
    version: "0.3.0",
    notes: "Ranch v0.3.0",
    pub_date: "2026-10-27T12:00:00.000Z",
    platforms: {
      "darwin-aarch64": {
        signature: "sig of Ranch.app.tar.gz",
        url: "https://github.com/Bramvanosta/ranch/releases/download/v0.3.0/Ranch.app.tar.gz",
      },
      "darwin-x86_64": {
        signature: "sig of Ranch.app.tar.gz",
        url: "https://github.com/Bramvanosta/ranch/releases/download/v0.3.0/Ranch.app.tar.gz",
      },
      "linux-x86_64": {
        signature: "sig of Ranch_0.3.0_amd64.AppImage",
        url: "https://github.com/Bramvanosta/ranch/releases/download/v0.3.0/Ranch_0.3.0_amd64.AppImage",
      },
    },
  })
  // The version's release: archives (one uploaded here, one by the job) and platform files, no leftovers.
  assert.deepEqual(names(gh, "v0.3.0"), [
    "Ranch.app.tar.gz",
    "Ranch_0.3.0_amd64.AppImage",
    "platform-darwin-aarch64.json",
    "platform-darwin-x86_64.json",
    "platform-linux-x86_64.json",
  ])
  assert.equal(
    gh.releases.get("v0.3.0").assets.find((a) => a.name.endsWith("AppImage")).body,
    "built",
  )
  // The feed: created once, a pre-release never marked latest, one asset per channel.
  assert.equal(gh.created.length, 1)
  assert.equal(gh.created[0].prerelease, true)
  assert.equal(gh.created[0].make_latest, "false")
  assert.deepEqual(names(gh, "update-feed"), ["latest.json", "rc.json"])
})

test("a pre-release moves rc.json only; no channel ever goes back", async () => {
  const gh = fakeGitHub()
  gh.addRelease("v0.3.0-rc.1")
  gh.addRelease("v0.3.0")
  gh.addRelease("v0.3.1-rc.1")

  await publish(gh, "v0.3.0-rc.1", ["linux-x86_64"], "a.AppImage")
  assert.equal(gh.feed("rc.json").version, "0.3.0-rc.1")
  assert.equal(gh.feed("latest.json"), undefined)

  await publish(gh, "v0.3.0", ["linux-x86_64"], "b.AppImage")
  assert.equal(gh.feed("rc.json").version, "0.3.0")
  assert.equal(gh.feed("latest.json").version, "0.3.0")

  // A re-run of the old RC: both channels stay on 0.3.0.
  await publish(gh, "v0.3.0-rc.1", ["linux-x86_64"], "a.AppImage")
  assert.equal(gh.feed("rc.json").version, "0.3.0")
  assert.equal(gh.feed("latest.json").version, "0.3.0")

  await publish(gh, "v0.3.1-rc.1", ["linux-x86_64"], "c.AppImage")
  assert.equal(gh.feed("rc.json").version, "0.3.1-rc.1")
  assert.equal(gh.feed("latest.json").version, "0.3.0")
})

test("a re-run heals a manifest that lost the race", async () => {
  const gh = fakeGitHub()
  const release = gh.addRelease("v0.3.0")
  const feed = gh.addRelease("update-feed", { prerelease: true })
  // Both platform files are there, but the job that wrote rc.json last had seen only its own.
  for (const p of ["linux-x86_64", "darwin-aarch64"])
    await gh.upload(release, `platform-${p}.json`, JSON.stringify({ signature: p, url: p }))
  await gh.upload(
    feed,
    "rc.json",
    JSON.stringify({ version: "0.3.0", platforms: { "linux-x86_64": {} } }),
  )
  // The archive is missing from the release: this run uploads it.
  await publish(gh, "v0.3.0", ["linux-x86_64"], "Ranch_0.3.0_amd64.AppImage")
  assert.deepEqual(Object.keys(gh.feed("rc.json").platforms), ["darwin-aarch64", "linux-x86_64"])
  assert.equal(
    gh.releases.get("v0.3.0").assets.find((a) => a.name === "Ranch_0.3.0_amd64.AppImage").body,
    "bytes of Ranch_0.3.0_amd64.AppImage",
  )
  assert.equal(gh.created.length, 0)
})

test("no release for the tag: it says so instead of publishing a feed", async () => {
  const gh = fakeGitHub()
  await assert.rejects(
    publish(gh, "v0.3.0", ["linux-x86_64"], "a.AppImage"),
    /no release for v0.3.0/,
  )
  assert.equal(gh.releases.has("update-feed"), false)
})

test("downloads follow GitHub's redirect without sending the token there", async () => {
  const calls = []
  const fetch = async (url, init = {}) => {
    calls.push({ url, authorization: init.headers?.authorization })
    if (url.endsWith("/releases/assets/42"))
      return new Response(null, {
        status: 302,
        headers: { location: "https://objects.example/x?sig=1" },
      })
    return new Response('{"version":"0.3.0"}', { status: 200 })
  }
  const gh = createGitHub({ token: "t0ken", repository: "o/r", fetch })
  assert.equal(await gh.download({ id: 42, name: "rc.json" }), '{"version":"0.3.0"}')
  assert.deepEqual(calls, [
    { url: "https://api.github.com/repos/o/r/releases/assets/42", authorization: "Bearer t0ken" },
    { url: "https://objects.example/x?sig=1", authorization: undefined },
  ])
})
