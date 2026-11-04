// node --test scripts/   (run by `pnpm test`)
//
// scripts/attach-release.sh against a `gh` whose release is a folder, and a
// `node` standing in for minisign.mjs's verify (FAKE_VERIFY=fail refuses):
// minisign.test.mjs tests the verification itself.
import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { after, test } from "node:test"

import { fakeBin } from "./fixtures/fake-tools.mjs"

const SCRIPT = join(import.meta.dirname, "attach-release.sh")
const tmp = mkdtempSync(join(tmpdir(), "ranch-attach-"))
after(() => rmSync(tmp, { recursive: true, force: true }))
let n = 0
const fresh = (name) => {
  const d = join(tmp, `${name}-${n++}`)
  mkdirSync(d, { recursive: true })
  return d
}
const sha256 = (data) => createHash("sha256").update(data).digest("hex")

/** A release (a folder) and the tools that reach it. */
function world(assets = {}) {
  const release = fresh("release")
  for (const [name, content] of Object.entries(assets)) writeFileSync(join(release, name), content)
  const fakes = fakeBin(fresh("fakes"), {
    gh: `
const opt = (f) => argv.filter((x, i) => argv[i - 1] === f)
const rel = env.FAKE_RELEASE_DIR
if (argv[1] === "view") for (const f of fs.readdirSync(rel).sort()) console.log(f)
else if (argv[1] === "download") for (const p of opt("--pattern")) {
  if (fs.existsSync(path.join(rel, p))) fs.copyFileSync(path.join(rel, p), path.join(opt("--dir")[0], p))
}
else if (argv[1] === "upload") for (const f of argv.slice(3).filter((a) => !a.startsWith("--"))) fs.copyFileSync(f, path.join(rel, path.basename(f)))`,
    node: `process.exit(env.FAKE_VERIFY === "fail" ? 1 : 0)`,
  })
  const attach = (platform, files, env = {}) => {
    const dir = fresh(`signed-${platform}`)
    for (const [name, content] of Object.entries(files)) writeFileSync(join(dir, name), content)
    const r = spawnSync("/bin/bash", [SCRIPT, platform, dir], {
      encoding: "utf8",
      env: { PATH: fakes.path(), GITHUB_REF_NAME: "v0.5.0", FAKE_RELEASE_DIR: release, ...env },
    })
    return { ...r, dir }
  }
  const uploads = () =>
    fakes
      .calls()
      .filter((c) => c.tool === "gh" && c.argv[1] === "upload")
      .map((c) =>
        c.argv
          .slice(3)
          .filter((a) => !a.startsWith("--"))
          .map((f) => f.split("/").at(-1))
          .sort(),
      )
  const asset = (name) => readFileSync(join(release, name), "utf8")
  return { release, fakes, attach, uploads, asset }
}

const MAC = {
  "Ranch_0.5.0_universal.dmg": "dmg-1",
  "Ranch.app.tar.gz": "archive-1",
  "Ranch.app.tar.gz.sig": "sig-1",
}
const LINUX = {
  "Ranch_0.5.0_amd64.deb": "deb-1",
  "Ranch-0.5.0-1.x86_64.rpm": "rpm-1",
  "Ranch_0.5.0_amd64.AppImage": "appimage-1",
  "Ranch_0.5.0_amd64.AppImage.sig": "appimage-sig-1",
}

test("before the feed serves it: every file uploaded with --clobber, with SHA256SUMS of exactly those files", () => {
  const w = world()
  const r = w.attach("macos", MAC)
  assert.equal(r.status, 0, r.stderr)
  assert.deepEqual(w.uploads(), [
    [
      "Ranch.app.tar.gz",
      "Ranch.app.tar.gz.sig",
      "Ranch_0.5.0_universal.dmg",
      "SHA256SUMS-macos.txt",
    ],
  ])
  assert.ok(
    w.fakes
      .calls()
      .find((c) => c.argv[1] === "upload")
      .argv.includes("--clobber"),
  )
  assert.equal(
    w.asset("SHA256SUMS-macos.txt"),
    `${sha256("archive-1")}  Ranch.app.tar.gz\n${sha256("sig-1")}  Ranch.app.tar.gz.sig\n${sha256("dmg-1")}  Ranch_0.5.0_universal.dmg\n`,
  )
  assert.deepEqual(
    w.fakes.calls().filter((c) => c.tool === "node"),
    [],
  )
})

test("a re-run before the feed serves it replaces every file (public release or not)", () => {
  const w = world({ ...MAC, "SHA256SUMS-macos.txt": "old" })
  const r = w.attach("macos", {
    "Ranch_0.5.0_universal.dmg": "dmg-2",
    "Ranch.app.tar.gz": "archive-2",
    "Ranch.app.tar.gz.sig": "sig-2",
  })
  assert.equal(r.status, 0, r.stderr)
  assert.equal(w.asset("Ranch.app.tar.gz"), "archive-2")
  assert.equal(w.asset("Ranch_0.5.0_universal.dmg"), "dmg-2")
})

for (const [platform, files, feed, archive] of [
  ["macos", MAC, "platform-darwin-aarch64.json", "Ranch.app.tar.gz"],
  ["macos", MAC, "platform-darwin-x86_64.json", "Ranch.app.tar.gz"],
  ["linux", LINUX, "platform-linux-x86_64.json", "Ranch_0.5.0_amd64.AppImage"],
]) {
  test(`${platform}, once ${feed} is published: a re-run keeps the archive and .sig the feed serves, replaces the rest`, () => {
    const w = world({ ...files, [feed]: "{}" })
    const rerun = Object.fromEntries(
      Object.entries(files).map(([k, v]) => [k, v.replace("-1", "-2")]),
    )
    const r = w.attach(platform, rerun)
    assert.equal(r.status, 0, r.stderr)
    assert.equal(w.asset(archive), files[archive])
    assert.equal(w.asset(`${archive}.sig`), files[`${archive}.sig`])
    for (const f of Object.keys(files).filter((x) => !x.startsWith(archive)))
      assert.equal(w.asset(f), rerun[f], f)
    assert.ok(!w.uploads()[0].includes(archive) && !w.uploads()[0].includes(`${archive}.sig`))
    const sums = w.asset(`SHA256SUMS-${platform}.txt`)
    assert.match(sums, new RegExp(`${sha256(files[archive])}  ${archive.replace(/\./g, "\\.")}\n`))
    assert.match(
      sums,
      new RegExp(`${sha256(files[`${archive}.sig`])}  ${archive.replace(/\./g, "\\.")}\\.sig\n`),
    )
    assert.match(
      r.stdout,
      new RegExp(
        `::notice::${archive.replace(/\./g, "\\.")}: the feed already serves the published one`,
      ),
    )
    const verify = w.fakes.calls().filter((c) => c.tool === "node")
    assert.equal(verify.length, 1)
    assert.deepEqual(verify[0].argv.slice(1, 2), ["verify"])
  })
}

test("once the feed serves it, a build without the updater still keeps the published archive in the sums", () => {
  const w = world({ ...MAC, "platform-darwin-aarch64.json": "{}" })
  const r = w.attach("macos", { "Ranch_0.5.0_universal.dmg": "dmg-2" })
  assert.equal(r.status, 0, r.stderr)
  assert.match(w.asset("SHA256SUMS-macos.txt"), /Ranch\.app\.tar\.gz\n/)
  assert.equal(w.asset("Ranch.app.tar.gz"), "archive-1")
})

test("the published pair must verify, and must be there: otherwise nothing is uploaded", () => {
  const bad = world({ ...LINUX, "platform-linux-x86_64.json": "{}" })
  const r = bad.attach("linux", LINUX, { FAKE_VERIFY: "fail" })
  assert.equal(r.status, 1)
  assert.match(
    r.stdout,
    /::error::attach-release: the published Ranch_0\.5\.0_amd64\.AppImage does not match its published signature/,
  )
  assert.deepEqual(bad.uploads(), [])
  const { "Ranch_0.5.0_amd64.AppImage.sig": _, ...noSig } = LINUX
  const gone = world({ ...noSig, "platform-linux-x86_64.json": "{}" })
  const g = gone.attach("linux", LINUX)
  assert.equal(g.status, 1)
  assert.match(g.stdout, /platform-linux-x86_64\.json is published but .* is not on the release/)
  assert.deepEqual(gone.uploads(), [])
})

test("the whole re-run: attach, the feed published, the signing jobs re-run, attach again — the feed's archive and signature unchanged", () => {
  const w = world()
  assert.equal(w.attach("linux", LINUX).status, 0)
  // publish: the feed now serves the AppImage and the signature on the release.
  writeFileSync(
    join(w.release, "platform-linux-x86_64.json"),
    JSON.stringify({ signature: w.asset("Ranch_0.5.0_amd64.AppImage.sig") }),
  )
  const served = {
    appimage: w.asset("Ranch_0.5.0_amd64.AppImage"),
    sig: w.asset("Ranch_0.5.0_amd64.AppImage.sig"),
  }
  const again = w.attach(
    "linux",
    Object.fromEntries(Object.entries(LINUX).map(([k, v]) => [k, v.replace("-1", "-2")])),
  )
  assert.equal(again.status, 0, again.stderr)
  assert.deepEqual(
    {
      appimage: w.asset("Ranch_0.5.0_amd64.AppImage"),
      sig: w.asset("Ranch_0.5.0_amd64.AppImage.sig"),
    },
    served,
  )
  assert.equal(JSON.parse(w.asset("platform-linux-x86_64.json")).signature, served.sig)
  assert.equal(w.asset("Ranch_0.5.0_amd64.deb"), "deb-2")
  assert.deepEqual(
    readdirSync(w.release).sort(),
    [...Object.keys(LINUX), "SHA256SUMS-linux.txt", "platform-linux-x86_64.json"].sort(),
  )
})

test("a wrong platform or folder is refused", () => {
  const w = world()
  assert.equal(w.attach("windows", {}).status, 1)
  const r = spawnSync("/bin/bash", [SCRIPT, "linux", join(tmp, "nope")], {
    encoding: "utf8",
    env: { PATH: w.fakes.path(), GITHUB_REF_NAME: "v0.5.0" },
  })
  assert.equal(r.status, 1)
})
