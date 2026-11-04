#!/usr/bin/env node
// Release pipelines: from the app the `build-macos` job made (universal,
// unsigned or ad hoc) to what a release ships.
//
//   node scripts/sign-macos.mjs --app <dir>/Ranch.app --out <dir> --version <version>
//        [--release] [--updater-archive]
//
// release.yml's `sign-macos` job runs it on a fresh runner that installs no
// dependency: its imports are Node.js built-ins, nothing else (a test walks
// them), and it runs only the system's tools (codesign, xcrun, hdiutil, spctl,
// ditto, xattr, /usr/bin/tar). Nothing of the app it was handed ever runs.
//
// - `--release`: signed inside-out with the Developer ID (APPLE_SIGNING_IDENTITY,
//   from the job's temporary keychain APPLE_KEYCHAIN), hardened runtime and a
//   secure timestamp; verified; notarized as a ditto zip with an App Store
//   Connect API key (APPLE_API_KEY, APPLE_API_ISSUER, the .p8 at
//   APPLE_API_KEY_PATH) and stapled. Then the DMG, signed, notarized and
//   stapled, and the checks: codesign --verify --deep --strict, Gatekeeper
//   (`source=Notarized Developer ID`), both tickets, APPLE_TEAM_ID everywhere.
//   Without it: signed ad hoc, and an unsigned DMG.
// - `--updater-archive`: Ranch.app.tar.gz, the archive the updater installs,
//   made from the app as it ships (stapled, in a release) by /usr/bin/tar: the
//   links kept, no extended attribute, no AppleDouble file. Its signature is
//   scripts/minisign.mjs's, in a step of its own.
//
// The app is signed with what Tauri's own signing gave it, nothing more: no
// entitlements (tauri.conf.json sets none; `codesign -d --entitlements -` on
// the 0.4.0 release prints none), the hardened runtime, the identifier of each
// nested binary its file name (`ranch-mcp`). A tauri.conf.json that sets
// entitlements or turns the hardened runtime off is refused rather than
// silently ignored.
import { spawnSync } from "node:child_process"
import {
  closeSync,
  existsSync,
  lstatSync,
  mkdirSync,
  openSync,
  readdirSync,
  readFileSync,
  readSync,
  rmSync,
  symlinkSync,
} from "node:fs"
import { basename, dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const REPO = join(dirname(fileURLToPath(import.meta.url)), "..")
export const PRODUCT = "Ranch"
export const UPDATE_ARCHIVE = `${PRODUCT}.app.tar.gz`
/** Notarization waits this long for Apple's answer, per submission. */
const NOTARY_TIMEOUT = "2h"
/** The APPLE_* names a release needs. */
export const RELEASE_ENV = [
  "APPLE_SIGNING_IDENTITY",
  "APPLE_TEAM_ID",
  "APPLE_KEYCHAIN",
  "APPLE_API_KEY",
  "APPLE_API_ISSUER",
  "APPLE_API_KEY_PATH",
]

export class SignError extends Error {}

export const dmgName = (version) => `${PRODUCT}_${version}_universal.dmg`

/** A tool of the system through PATH; throws with what it said unless `ok` accepts the result. */
export function run(cmd, args, { env = process.env, ok = (r) => r.status === 0, input } = {}) {
  const r = spawnSync(cmd, args, { encoding: "utf8", env, input, maxBuffer: 64 * 1024 * 1024 })
  if (r.error) throw new SignError(`${cmd}: ${r.error.message}`)
  if (!ok(r)) {
    const said = `${r.stdout ?? ""}${r.stderr ?? ""}`.trim().split("\n").slice(-8).join("\n  ")
    throw new SignError(`${cmd} ${args[0]} failed (exit ${r.status}):\n  ${said}`)
  }
  return r
}

const sleep = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms)

/** hdiutil, tried again up to `retries` times when, and only when, it says "Resource busy". */
export function hdiutil(args, { env = process.env, retries = 3, delayMs = 5000 } = {}) {
  for (let attempt = 0; ; attempt++) {
    const r = spawnSync("hdiutil", args, { encoding: "utf8", env })
    if (r.error) throw new SignError(`hdiutil: ${r.error.message}`)
    if (r.status === 0) return r
    const said = `${r.stdout ?? ""}${r.stderr ?? ""}`
    if (!/Resource busy/.test(said) || attempt >= retries) {
      throw new SignError(
        `hdiutil ${args[0]} failed (exit ${r.status}):\n  ${said.trim().split("\n").slice(-8).join("\n  ")}`,
      )
    }
    console.error(
      `sign-macos: hdiutil ${args[0]}: Resource busy, trying again (${attempt + 1}/${retries})`,
    )
    sleep(delayMs)
  }
}

/** A key of the app's Info.plist (Tauri writes it as XML). */
export function plistValue(app, key) {
  const xml = readFileSync(join(app, "Contents", "Info.plist"), "utf8")
  const m = new RegExp(`<key>${key}</key>\\s*<string>([^<]*)</string>`).exec(xml)
  if (!m) throw new SignError(`${basename(app)}: no ${key} in Contents/Info.plist`)
  return m[1]
}

/** Refuses a tauri.conf.json asking for a signature this script does not make. */
export function checkTauriConf(
  conf = JSON.parse(readFileSync(join(REPO, "src-tauri", "tauri.conf.json"), "utf8")),
) {
  const mac = conf.bundle?.macOS ?? {}
  if (mac.entitlements) {
    throw new SignError(
      "tauri.conf.json sets bundle.macOS.entitlements: sign-macos.mjs signs without any, so teach it them first",
    )
  }
  if (mac.hardenedRuntime === false) {
    throw new SignError(
      "tauri.conf.json turns the hardened runtime off: sign-macos.mjs always signs with it",
    )
  }
}

const MACHO = new Set(["feedface", "feedfacf", "cefaedfe", "cffaedfe", "cafebabe", "bebafeca"])
function isMachO(file) {
  const fd = openSync(file, "r")
  const b = Buffer.alloc(4)
  const n = readSync(fd, b, 0, 4, 0)
  closeSync(fd)
  return n === 4 && MACHO.has(b.toString("hex"))
}

/**
 * The code nested in the app, deepest first: every Mach-O file of Contents
 * but the main executable (the app's own signature covers it). A nested
 * bundle (framework, app, plug-in) is refused: none ships today, and each
 * would need signing as a bundle.
 */
export function nestedCode(app) {
  const main = join(app, "Contents", "MacOS", plistValue(app, "CFBundleExecutable"))
  const found = []
  const walk = (dir, depth) => {
    for (const name of readdirSync(dir).sort()) {
      const p = join(dir, name)
      const st = lstatSync(p)
      if (st.isSymbolicLink()) continue
      if (st.isDirectory()) {
        if (/\.(framework|app|appex|bundle|xpc|plugin)$/.test(name)) {
          throw new SignError(
            `${p.slice(app.length + 1)}: a nested bundle; sign-macos.mjs signs plain binaries only`,
          )
        }
        walk(p, depth + 1)
      } else if (st.isFile() && p !== main && isMachO(p)) found.push({ path: p, depth })
    }
  }
  walk(join(app, "Contents"), 0)
  if (!existsSync(main))
    throw new SignError(`${basename(app)}: no main executable ${main.slice(app.length + 1)}`)
  return found.sort((a, b) => b.depth - a.depth || a.path.localeCompare(b.path)).map((f) => f.path)
}

/** The signer of a mode: what codesign and notarytool are given. Refuses a release without every APPLE_* name. */
export function signer(release, env = process.env) {
  if (!release) return { release: false, identity: "-" }
  const missing = RELEASE_ENV.filter((k) => !env[k])
  if (missing.length)
    throw new SignError(`a release needs ${missing.join(", ")} — nothing was signed`)
  if (!existsSync(env.APPLE_API_KEY_PATH))
    throw new SignError("APPLE_API_KEY_PATH names no file — nothing was signed")
  return {
    release: true,
    identity: env.APPLE_SIGNING_IDENTITY,
    team: env.APPLE_TEAM_ID,
    keychain: env.APPLE_KEYCHAIN,
    notary: [
      "--key",
      env.APPLE_API_KEY_PATH,
      "--key-id",
      env.APPLE_API_KEY,
      "--issuer",
      env.APPLE_API_ISSUER,
    ],
  }
}

/** codesign's arguments for one item. */
export function codesignArgs(s, path, { identifier } = {}) {
  const args = ["--force", "--sign", s.identity, "--options", "runtime"]
  if (s.release) args.push("--timestamp", "--keychain", s.keychain)
  if (identifier) args.push("--identifier", identifier)
  return [...args, path]
}

/** Inside-out: each nested binary, then the app. Returns what was signed, in order. */
export function signApp(app, s, { env } = {}) {
  const items = nestedCode(app)
  for (const p of items) run("codesign", codesignArgs(s, p, { identifier: basename(p) }), { env })
  run("codesign", codesignArgs(s, app), { env })
  run("codesign", ["--verify", "--deep", "--strict", "--verbose=2", app], { env })
  return [...items, app]
}

/** One notarization, waited for; its log said when Apple refuses it. Then the ticket stapled and checked. */
export function notarize(s, file, staple, { env } = {}) {
  const r = run(
    "xcrun",
    [
      "notarytool",
      "submit",
      file,
      ...s.notary,
      "--wait",
      "--timeout",
      NOTARY_TIMEOUT,
      "--output-format",
      "json",
    ],
    {
      env,
      ok: () => true,
    },
  )
  let answer = {}
  try {
    answer = JSON.parse(r.stdout)
  } catch {}
  if (answer.status !== "Accepted") {
    if (answer.id) {
      const log = run("xcrun", ["notarytool", "log", answer.id, ...s.notary], {
        env,
        ok: () => true,
      })
      console.error(log.stdout)
    }
    throw new SignError(
      `notarization of ${basename(file)}: ${answer.status ?? "no answer"} ${answer.message ?? (r.stderr || "").trim()}`,
    )
  }
  run("xcrun", ["stapler", "staple", staple], { env })
  run("xcrun", ["stapler", "validate", staple], { env })
}

/** The updater's archive of the app: /usr/bin/tar, links kept, no extended attribute, no ._ file. */
export function makeUpdateArchive(app, out, { env = process.env, tar = "/usr/bin/tar" } = {}) {
  const archive = join(out, UPDATE_ARCHIVE)
  rmSync(archive, { force: true })
  run(
    tar,
    ["--no-mac-metadata", "--no-xattrs", "-czf", archive, "-C", dirname(app), basename(app)],
    {
      env: { ...env, COPYFILE_DISABLE: "1" },
    },
  )
  return archive
}

/** The DMG: the app and a link to /Applications, made by hdiutil (not Tauri's bundle_dmg.sh). */
export function makeDmg(app, out, version, s, { env, retry } = {}) {
  const stage = join(out, "dmg-root")
  rmSync(stage, { recursive: true, force: true })
  mkdirSync(stage, { recursive: true })
  run("ditto", [app, join(stage, basename(app))], { env })
  symlinkSync("/Applications", join(stage, "Applications"))
  const dmg = join(out, dmgName(version))
  rmSync(dmg, { force: true })
  hdiutil(
    [
      "create",
      "-volname",
      PRODUCT,
      "-srcfolder",
      stage,
      "-fs",
      "HFS+",
      "-format",
      "UDZO",
      "-ov",
      dmg,
    ],
    { env, ...retry },
  )
  rmSync(stage, { recursive: true, force: true })
  hdiutil(["verify", dmg], { env, ...retry })
  if (s.release)
    run(
      "codesign",
      ["--force", "--sign", s.identity, "--timestamp", "--keychain", s.keychain, dmg],
      { env },
    )
  return dmg
}

/** What a release must be once done; returns the problems found (none: it ships). */
export function releaseProblems(app, dmg, items, s, { env } = {}) {
  const problems = []
  const said = (cmd, args) => {
    const r = spawnSync(cmd, args, { encoding: "utf8", env })
    return { ok: r.status === 0, text: `${r.stdout ?? ""}${r.stderr ?? ""}` }
  }
  if (!said("codesign", ["--verify", "--deep", "--strict", "--verbose=2", app]).ok)
    problems.push("codesign --verify --deep --strict refuses the app")
  for (const p of [...items, dmg]) {
    const d = said("codesign", ["-dvv", p])
    if (!d.text.includes(`TeamIdentifier=${s.team}`))
      problems.push(`${basename(p)}: not signed by team ${s.team}`)
    if (!/Authority=Developer ID Application: /.test(d.text))
      problems.push(`${basename(p)}: not signed with a Developer ID Application certificate`)
    if (p !== dmg && !/flags=0x[0-9a-f]*\(runtime\)/.test(d.text))
      problems.push(`${basename(p)}: no hardened runtime`)
  }
  const gatekeeper = [
    ["app", ["--assess", "--type", "execute", "-vv", app]],
    ["DMG", ["--assess", "--type", "open", "--context", "context:primary-signature", "-vv", dmg]],
  ]
  for (const [what, args] of gatekeeper) {
    const g = said("spctl", args)
    if (!g.ok || !/: accepted/.test(g.text) || !/source=Notarized Developer ID/.test(g.text)) {
      problems.push(
        `Gatekeeper does not accept the ${what} as notarized (${g.text.trim().split("\n").join("; ")})`,
      )
    }
  }
  for (const p of [app, dmg])
    if (!said("xcrun", ["stapler", "validate", p]).ok)
      problems.push(`${basename(p)}: no valid stapled ticket`)
  return problems
}

/** The whole job; see the module comment. Returns `{ dmg, archive }`. */
export function signRelease({
  app,
  out,
  version,
  release,
  updaterArchive,
  env = process.env,
  retry,
  conf,
}) {
  checkTauriConf(conf)
  const s = signer(release, env)
  if (!existsSync(join(app, "Contents", "Info.plist")))
    throw new SignError(`--app ${app}: not an app bundle`)
  mkdirSync(out, { recursive: true })
  if (release) run("xattr", ["-cr", app], { env })
  const items = signApp(app, s, { env })
  console.error(
    `sign-macos: ${items.length} signatures (${release ? "Developer ID" : "ad hoc"}), verified`,
  )
  if (release) {
    const zip = join(out, `${PRODUCT}-notarize.zip`)
    run("ditto", ["-c", "-k", "--keepParent", app, zip], { env })
    notarize(s, zip, app, { env })
    rmSync(zip, { force: true })
    console.error("sign-macos: app notarized and stapled")
  }
  const archive = updaterArchive ? makeUpdateArchive(app, out, { env }) : null
  const dmg = makeDmg(app, out, version, s, { env, retry })
  if (release) {
    notarize(s, dmg, dmg, { env })
    const problems = releaseProblems(app, dmg, items, s, { env })
    if (problems.length)
      throw new SignError(`the release fails its checks:\n  ${problems.join("\n  ")}`)
    console.error("sign-macos: DMG notarized and stapled; Gatekeeper, tickets and team checked")
  } else {
    console.error("sign-macos: an ad hoc build: not signed with a Developer ID, not notarized")
  }
  return { dmg, archive }
}

export function parseArgs(argv) {
  const out = { release: false, updaterArchive: false }
  const names = { "--app": "app", "--out": "out", "--version": "version" }
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (a === "--release") out.release = true
    else if (a === "--updater-archive") out.updaterArchive = true
    else if (names[a] && argv[i + 1] && !argv[i + 1].startsWith("--")) out[names[a]] = argv[++i]
    else throw new SignError(`unknown argument "${a}"`)
  }
  for (const k of ["app", "out", "version"])
    if (!out[k]) throw new SignError(`sign-macos.mjs needs --${k}`)
  return { ...out, app: resolve(out.app), out: resolve(out.out) }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    signRelease(parseArgs(process.argv.slice(2)))
  } catch (e) {
    console.error(`sign-macos: ${e instanceof SignError ? e.message : (e?.stack ?? e)}`)
    process.exit(1)
  }
}
