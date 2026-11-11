// node --test scripts/   (run by `pnpm test`)
//
// scripts/sign-macos.mjs against stand-ins: codesign, xcrun (notarytool,
// stapler), hdiutil, spctl, ditto and xattr are fakes on a PATH that holds
// nothing else, so no real tool, keychain or Apple service is ever reached.
// The fakes answer as a good release would; each check is then shown to bite
// by making one fake answer otherwise.
import assert from "node:assert/strict"
import {
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readlinkSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { after, test } from "node:test"

import { fakeBin } from "./fixtures/fake-tools.mjs"
import {
  checkTauriConf,
  dmgName,
  hdiutil,
  nestedCode,
  RELEASE_ENV,
  signRelease,
} from "./sign-macos.mjs"

const tmp = mkdtempSync(join(tmpdir(), "ranch-sign-macos-"))
after(() => rmSync(tmp, { recursive: true, force: true }))
let n = 0
const fresh = (name) => {
  const d = join(tmp, `${name}-${n++}`)
  mkdirSync(d, { recursive: true })
  return d
}

const MACHO = Buffer.from("cffaedfe07000001", "hex")
const PLIST = `<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key>
  <string>ranch</string>
  <key>CFBundleIdentifier</key>
  <string>dev.ranch.app</string>
  <key>CFBundleShortVersionString</key>
  <string>0.5.0</string>
</dict></plist>
`

/** An app laid out as `tauri bundle` lays Ranch out: two binaries, an icon. */
function fakeApp(dir) {
  const app = join(dir, "Ranch.app")
  mkdirSync(join(app, "Contents", "MacOS"), { recursive: true })
  mkdirSync(join(app, "Contents", "Resources"), { recursive: true })
  writeFileSync(join(app, "Contents", "Info.plist"), PLIST)
  writeFileSync(join(app, "Contents", "MacOS", "ranch"), MACHO)
  writeFileSync(join(app, "Contents", "MacOS", "ranch-mcp"), MACHO)
  writeFileSync(join(app, "Contents", "Resources", "icon.icns"), "icns")
  return app
}

/**
 * The tools, answering as for a good release. FAKE_* in the environment make
 * one answer otherwise: FAKE_TEAM (codesign -dvv's team), FAKE_NOTARY (the
 * notarization status), FAKE_SPCTL (Gatekeeper's source), FAKE_VERIFY=fail,
 * FAKE_BUSY (how many hdiutil runs say "Resource busy" first), FAKE_HDIUTIL_ERROR.
 */
function tools(dir) {
  return fakeBin(dir, {
    codesign: `
if (argv[0] === "--verify") process.exit(env.FAKE_VERIFY === "fail" ? 1 : 0)
if (argv[0] === "-dvv") {
  console.error("Authority=Developer ID Application: Test (" + (env.FAKE_TEAM || "TEAM123") + ")")
  console.error("TeamIdentifier=" + (env.FAKE_TEAM || "TEAM123"))
  console.error("CodeDirectory v=20500 size=1 flags=0x10000(runtime) hashes=1+1")
}`,
    xcrun: `
if (argv[0] === "notarytool" && argv[1] === "submit") {
  console.log(JSON.stringify({ id: "sub-1", status: env.FAKE_NOTARY || "Accepted", message: "done" }))
} else if (argv[0] === "notarytool" && argv[1] === "log") console.log("{ \\"issues\\": [] }")`,
    hdiutil: `
const n = Number(fs.existsSync(state + "/busy") ? fs.readFileSync(state + "/busy", "utf8") : 0)
if (n < Number(env.FAKE_BUSY || 0)) { fs.writeFileSync(state + "/busy", String(n + 1)); console.error("hdiutil: create failed - Resource busy"); process.exit(1) }
if (env.FAKE_HDIUTIL_ERROR) { console.error(env.FAKE_HDIUTIL_ERROR); process.exit(1) }
if (argv[0] === "create") {
  const src = argv[argv.indexOf("-srcfolder") + 1]
  const entries = fs.readdirSync(src).sort().map((e) => e + (fs.lstatSync(path.join(src, e)).isSymbolicLink() ? " -> " + fs.readlinkSync(path.join(src, e)) : ""))
  fs.writeFileSync(state + "/dmg-content.json", JSON.stringify(entries))
  fs.writeFileSync(argv[argv.length - 1], "a dmg")
}`,
    spctl: `console.error(argv[argv.length - 1] + ": accepted\\nsource=" + (env.FAKE_SPCTL || "Notarized Developer ID"))`,
    ditto: `
if (argv[0] === "-c") fs.writeFileSync(argv[argv.length - 1], "a zip")
else fs.cpSync(argv[0], argv[1], { recursive: true, verbatimSymlinks: true })`,
    xattr: "",
  })
}

function releaseEnv(dir, fakes, extra = {}) {
  const key = join(dir, "AuthKey.p8")
  writeFileSync(key, "-----BEGIN PRIVATE KEY-----\nfake\n-----END PRIVATE KEY-----\n", {
    mode: 0o600,
  })
  return {
    PATH: fakes.path(""),
    APPLE_SIGNING_IDENTITY: "Developer ID Application: Test (TEAM123)",
    APPLE_TEAM_ID: "TEAM123",
    APPLE_KEYCHAIN: join(dir, "ranch-signing.keychain-db"),
    APPLE_API_KEY: "KEYID123",
    APPLE_API_ISSUER: "issuer-uuid",
    APPLE_API_KEY_PATH: key,
    ...extra,
  }
}

const NO_WAIT = { retries: 3, delayMs: 0 }
const sign = (dir, env, more = {}) =>
  signRelease({
    app: fakeApp(dir),
    out: join(dir, "out"),
    version: "0.5.0",
    release: true,
    env,
    retry: NO_WAIT,
    conf: {},
    ...more,
  })
const argvOf = (calls, tool, first) =>
  calls.filter((c) => c.tool === tool && (!first || c.argv[0] === first)).map((c) => c.argv)

test("a release: inside-out, the Developer ID from the job's keychain, hardened runtime, timestamp, no entitlements", () => {
  const dir = fresh("release")
  const fakes = tools(dir)
  const { dmg } = sign(dir, releaseEnv(dir, fakes))
  const app = join(dir, "Ranch.app")
  const signs = argvOf(fakes.calls(), "codesign", "--force")
  const id = "Developer ID Application: Test (TEAM123)"
  const kc = join(dir, "ranch-signing.keychain-db")
  assert.deepEqual(signs, [
    [
      "--force",
      "--sign",
      id,
      "--options",
      "runtime",
      "--timestamp",
      "--keychain",
      kc,
      "--identifier",
      "ranch-mcp",
      join(app, "Contents", "MacOS", "ranch-mcp"),
    ],
    ["--force", "--sign", id, "--options", "runtime", "--timestamp", "--keychain", kc, app],
    ["--force", "--sign", id, "--timestamp", "--keychain", kc, dmg],
  ])
  assert.ok(!JSON.stringify(fakes.calls()).includes("--entitlements"))
  assert.deepEqual(argvOf(fakes.calls(), "xattr"), [["-cr", app]])
})

test("a release: the app notarized as a ditto zip with the API key (no password anywhere), stapled; then the DMG", () => {
  const dir = fresh("notary")
  const fakes = tools(dir)
  const { dmg } = sign(dir, releaseEnv(dir, fakes))
  const app = join(dir, "Ranch.app")
  const zip = join(dir, "out", "Ranch-notarize.zip")
  const creds = [
    "--key",
    join(dir, "AuthKey.p8"),
    "--key-id",
    "KEYID123",
    "--issuer",
    "issuer-uuid",
  ]
  const flow = fakes
    .calls()
    .filter((c) => c.tool === "xcrun" || (c.tool === "ditto" && c.argv[0] === "-c"))
    .map((c) => c.argv)
  assert.deepEqual(flow, [
    ["-c", "-k", "--keepParent", app, zip],
    ["notarytool", "submit", zip, ...creds, "--wait", "--timeout", "2h", "--output-format", "json"],
    ["stapler", "staple", app],
    ["stapler", "validate", app],
    ["notarytool", "submit", dmg, ...creds, "--wait", "--timeout", "2h", "--output-format", "json"],
    ["stapler", "staple", dmg],
    ["stapler", "validate", dmg],
    ["stapler", "validate", app],
    ["stapler", "validate", dmg],
  ])
  assert.ok(!/password|APPLE_ID|-p\b/.test(JSON.stringify(fakes.calls())))
})

test("the DMG: made by hdiutil (no bundle_dmg.sh) from the app and an Applications link, verified, named as Tauri named it", () => {
  const dir = fresh("dmg")
  const fakes = tools(dir)
  const { dmg } = sign(dir, releaseEnv(dir, fakes))
  assert.equal(dmg, join(dir, "out", dmgName("0.5.0")))
  assert.equal(dmgName("0.5.0-rc.1"), "Ranch_0.5.0-rc.1_universal.dmg")
  const [create, verify] = argvOf(fakes.calls(), "hdiutil")
  assert.deepEqual(create, [
    "create",
    "-volname",
    "Ranch",
    "-srcfolder",
    join(dir, "out", "dmg-root"),
    "-fs",
    "HFS+",
    "-format",
    "UDZO",
    "-ov",
    dmg,
  ])
  assert.deepEqual(verify, ["verify", dmg])
  assert.deepEqual(JSON.parse(readFileSync(join(fakes.state, "dmg-content.json"), "utf8")), [
    "Applications -> /Applications",
    "Ranch.app",
  ])
})

test("the final checks: deep strict verify, the team on every signature, Gatekeeper's notarized source for app and DMG", () => {
  const dir = fresh("checks")
  const fakes = tools(dir)
  const { dmg } = sign(dir, releaseEnv(dir, fakes))
  const app = join(dir, "Ranch.app")
  const calls = fakes.calls()
  assert.ok(
    argvOf(calls, "codesign", "--verify").every(
      (a) => a.join(" ") === `--verify --deep --strict --verbose=2 ${app}`,
    ),
  )
  assert.equal(argvOf(calls, "codesign", "--verify").length, 2)
  assert.deepEqual(
    argvOf(calls, "codesign", "-dvv").map((a) => a[1]),
    [join(app, "Contents", "MacOS", "ranch-mcp"), app, dmg],
  )
  assert.deepEqual(argvOf(calls, "spctl"), [
    ["--assess", "--type", "execute", "-vv", app],
    ["--assess", "--type", "open", "--context", "context:primary-signature", "-vv", dmg],
  ])
})

test("each check bites: another team, Gatekeeper without notarization, a failed verify, a refused notarization (its log fetched)", () => {
  const cases = [
    [
      { FAKE_TEAM: "OTHER" },
      /ranch-mcp: not signed by team TEAM123[\s\S]*Ranch\.app: not signed by team TEAM123/,
    ],
    [
      { FAKE_SPCTL: "Developer ID" },
      /Gatekeeper does not accept the app as notarized[\s\S]*the DMG as notarized/,
    ],
    [{ FAKE_VERIFY: "fail" }, /codesign --verify failed/],
    [{ FAKE_NOTARY: "Invalid" }, /notarization of Ranch-notarize\.zip: Invalid/],
  ]
  for (const [extra, message] of cases) {
    const dir = fresh("bite")
    const fakes = tools(dir)
    assert.throws(() => sign(dir, releaseEnv(dir, fakes, extra)), message, JSON.stringify(extra))
    if (extra.FAKE_NOTARY) {
      assert.deepEqual(argvOf(fakes.calls(), "xcrun").at(-1).slice(0, 3), [
        "notarytool",
        "log",
        "sub-1",
      ])
      assert.equal(argvOf(fakes.calls(), "hdiutil").length, 0)
    }
  }
})

test("a release without every APPLE_* name is refused before any tool runs, naming what is missing", () => {
  for (const name of RELEASE_ENV) {
    const dir = fresh("missing")
    const fakes = tools(dir)
    const env = releaseEnv(dir, fakes, { [name]: "" })
    assert.throws(() => sign(dir, env), new RegExp(`a release needs ${name} — nothing was signed`))
    assert.deepEqual(fakes.calls(), [])
  }
  const dir = fresh("nokey")
  const fakes = tools(dir)
  assert.throws(
    () => sign(dir, releaseEnv(dir, fakes, { APPLE_API_KEY_PATH: join(dir, "nope.p8") })),
    /names no file/,
  )
})

test("ad hoc (no Apple set): signed with `-`, no keychain, no timestamp, no notarization, an unsigned DMG", () => {
  const dir = fresh("adhoc")
  const fakes = tools(dir)
  sign(dir, { PATH: fakes.path("") }, { release: false })
  const app = join(dir, "Ranch.app")
  assert.deepEqual(argvOf(fakes.calls(), "codesign", "--force"), [
    [
      "--force",
      "--sign",
      "-",
      "--options",
      "runtime",
      "--identifier",
      "ranch-mcp",
      join(app, "Contents", "MacOS", "ranch-mcp"),
    ],
    ["--force", "--sign", "-", "--options", "runtime", app],
  ])
  assert.deepEqual(argvOf(fakes.calls(), "xcrun"), [])
  assert.deepEqual(argvOf(fakes.calls(), "spctl"), [])
  assert.deepEqual(argvOf(fakes.calls(), "xattr"), [])
})

test("hdiutil is tried again on Resource busy only, three times at most", () => {
  const busy = (times, extra = {}) => {
    const dir = fresh("busy")
    const fakes = tools(dir)
    const env = { PATH: fakes.path(""), FAKE_BUSY: String(times), ...extra }
    let error = null
    try {
      hdiutil(["create", "-srcfolder", dir, join(dir, "x.dmg")], { env, ...NO_WAIT })
    } catch (e) {
      error = e
    }
    return { error, runs: argvOf(fakes.calls(), "hdiutil").length }
  }
  assert.deepEqual(busy(0), { error: null, runs: 1 })
  assert.deepEqual(busy(3), { error: null, runs: 4 })
  const four = busy(4)
  assert.equal(four.runs, 4)
  assert.match(four.error.message, /Resource busy/)
  const other = busy(0, { FAKE_HDIUTIL_ERROR: "hdiutil: create failed - No space left on device" })
  assert.equal(other.runs, 1)
  assert.match(other.error.message, /No space left/)
})

test("the nested code: every Mach-O of Contents but the main executable; links skipped; a nested bundle refused", () => {
  const dir = fresh("nested")
  const app = fakeApp(dir)
  assert.deepEqual(nestedCode(app), [join(app, "Contents", "MacOS", "ranch-mcp")])
  mkdirSync(join(app, "Contents", "Resources", "bin"))
  writeFileSync(join(app, "Contents", "Resources", "bin", "helper"), MACHO)
  assert.deepEqual(nestedCode(app), [
    join(app, "Contents", "Resources", "bin", "helper"),
    join(app, "Contents", "MacOS", "ranch-mcp"),
  ])
  mkdirSync(join(app, "Contents", "Frameworks", "X.framework"), { recursive: true })
  assert.throws(() => nestedCode(app), /Contents\/Frameworks\/X\.framework: a nested bundle/)
})

test("tauri.conf.json: entitlements or no hardened runtime are refused, the repository's own is accepted", () => {
  assert.doesNotThrow(() => checkTauriConf())
  assert.throws(
    () => checkTauriConf({ bundle: { macOS: { entitlements: "e.plist" } } }),
    /entitlements/,
  )
  assert.throws(
    () => checkTauriConf({ bundle: { macOS: { hardenedRuntime: false } } }),
    /hardened runtime/,
  )
})

test("the updater archive (macOS's tar): the app as it ships, links kept, no AppleDouble or extended attribute", {
  skip: process.platform !== "darwin" && "/usr/bin/tar is not macOS's",
}, async () => {
  const { makeUpdateArchive } = await import("./sign-macos.mjs")
  const { spawnSync } = await import("node:child_process")
  const dir = fresh("archive")
  const app = fakeApp(dir)
  const { symlinkSync } = await import("node:fs")
  symlinkSync("MacOS/ranch", join(app, "Contents", "link"))
  spawnSync("/usr/bin/xattr", ["-w", "com.example.test", "x", join(app, "Contents", "Info.plist")])
  const out = fresh("archive-out")
  const archive = makeUpdateArchive(app, out)
  assert.equal(archive, join(out, "Ranch.app.tar.gz"))
  const list = spawnSync("/usr/bin/tar", ["-tzvf", archive], { encoding: "utf8" }).stdout
  assert.match(list, /Ranch\.app\/Contents\/link -> MacOS\/ranch/)
  assert.doesNotMatch(list, /\/\._|LIBARCHIVE\.xattr/)
  // The updater drops the first component of every entry and swaps what is
  // left in at the running app's path: one `Ranch.app/` holding everything,
  // no `./` before it, nothing beside it.
  const names = spawnSync("/usr/bin/tar", ["-tzf", archive], { encoding: "utf8" })
    .stdout.split("\n")
    .filter(Boolean)
  assert.ok(names.includes("Ranch.app/Contents/Info.plist"), names.join(", "))
  assert.deepEqual([...new Set(names.map((n) => n.split("/")[0]))], ["Ranch.app"])
  const x = fresh("archive-x")
  spawnSync("/usr/bin/tar", ["-xzf", archive, "-C", x])
  assert.equal(readlinkSync(join(x, "Ranch.app", "Contents", "link")), "MacOS/ranch")
  assert.ok(lstatSync(join(x, "Ranch.app", "Contents", "link")).isSymbolicLink())
  const attrs = spawnSync("/usr/bin/xattr", [join(x, "Ranch.app", "Contents", "Info.plist")], {
    encoding: "utf8",
  }).stdout
  // macOS adds com.apple.provenance itself on extraction; the archive's own attribute must not come back.
  assert.doesNotMatch(attrs, /com\.example\.test/)
})

test("the updater archive's command: /usr/bin/tar, no mac metadata, no xattrs, COPYFILE_DISABLE, the app alone", async () => {
  const { makeUpdateArchive } = await import("./sign-macos.mjs")
  const dir = fresh("tar-argv")
  const app = fakeApp(dir)
  const fakes = fakeBin(dir, {
    tar: `fs.writeFileSync(state + "/env.json", JSON.stringify(env.COPYFILE_DISABLE ?? null))`,
  })
  const archive = makeUpdateArchive(app, fresh("tar-out"), {
    env: { PATH: "" },
    tar: join(fakes.bin, "tar"),
  })
  assert.deepEqual(fakes.calls()[0].argv, [
    "--no-mac-metadata",
    "--no-xattrs",
    "-czf",
    archive,
    "-C",
    dir,
    "Ranch.app",
  ])
  assert.equal(JSON.parse(readFileSync(join(fakes.state, "env.json"), "utf8")), "1")
  const src = readFileSync(join(import.meta.dirname, "sign-macos.mjs"), "utf8")
  assert.match(
    src,
    /export function makeUpdateArchive\(app, out, \{ env = process\.env, tar = "\/usr\/bin\/tar" \} = \{\}\)/,
  )
  assert.match(
    src,
    /const archive = updaterArchive \? makeUpdateArchive\(app, out, \{ env \}\) : null/,
  )
})
