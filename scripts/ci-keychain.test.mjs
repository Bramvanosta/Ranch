// node --test scripts/   (run by `pnpm test`)
//
// scripts/ci-keychain.sh against a recording `security` (never the real one,
// never a real keychain): the certificate a throwaway .p12 made here by the
// system's openssl. The rules (`keychainProblems`) are checked on the real
// script, then each on a copy of it with that rule broken.
import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { after, describe, test } from "node:test"

import { fakeBin } from "./fixtures/fake-tools.mjs"

const SCRIPT = join(import.meta.dirname, "ci-keychain.sh")
const SOURCE = readFileSync(SCRIPT, "utf8")
const tmp = mkdtempSync(join(tmpdir(), "ranch-keychain-"))
after(() => rmSync(tmp, { recursive: true, force: true }))
let n = 0
const fresh = (name) => {
  const d = join(tmp, `${name}-${n++}`)
  mkdirSync(d, { recursive: true })
  return d
}

const IDENTITY = "Developer ID Application: Test (TEAM123)"
const LOGIN = "/Users/runner/Library/Keychains/login.keychain-db"
const PASSWORD = "p12-password-canary"

/** A .p12 of a self-signed certificate, base64, made by the system's openssl. */
const P12 = (() => {
  const d = fresh("p12")
  const o = (...args) => {
    const r = spawnSync("/usr/bin/openssl", args, { cwd: d, encoding: "utf8" })
    assert.equal(r.status, 0, r.stderr)
  }
  o(
    "req",
    "-x509",
    "-newkey",
    "rsa:2048",
    "-nodes",
    "-keyout",
    "k.pem",
    "-out",
    "c.pem",
    "-subj",
    "/CN=Test",
    "-days",
    "1",
  )
  o(
    "pkcs12",
    "-export",
    "-in",
    "c.pem",
    "-inkey",
    "k.pem",
    "-passout",
    `pass:${PASSWORD}`,
    "-out",
    "t.p12",
  )
  return readFileSync(join(d, "t.p12")).toString("base64")
})()

/**
 * A `security` that keeps a search list in its state folder and records each
 * call (`-i`: each line of its standard input as a call of its own).
 * FAKE_FAIL names a command that fails; FAKE_IDENTITY what find-identity lists.
 */
function fakeSecurity(dir) {
  return fakeBin(
    dir,
    {
      security: `
const list = state + "/list"
if (!fs.existsSync(list)) fs.writeFileSync(list, ${JSON.stringify(`${LOGIN}\n`)})
const lines = () => fs.readFileSync(list, "utf8").split("\\n").filter(Boolean)
const run = (a) => {
  if (a[0] === env.FAKE_FAIL) { console.error("security: " + a[0] + " failed"); return 1 }
  if (a[0] === "list-keychains" && a.includes("-s")) fs.writeFileSync(list, a.slice(a.indexOf("-s") + 1).map((k) => k + "\\n").join(""))
  else if (a[0] === "list-keychains") for (const k of lines()) console.log('    "' + k + '"')
  else if (a[0] === "create-keychain") { fs.writeFileSync(a.at(-1), "kc"); fs.writeFileSync(list, [...lines(), a.at(-1)].map((k) => k + "\\n").join("")) }
  else if (a[0] === "delete-keychain") fs.rmSync(a.at(-1)) // the search list as it was: putting it back is the script's job
  else if (a[0] === "find-identity") console.log('  1) ABCDEF "' + (env.FAKE_IDENTITY || ${JSON.stringify(IDENTITY)}) + '"\\n     1 valid identities found')
  return 0
}
if (argv[0] === "-i") {
  let status = 0
  for (const l of stdin.split("\\n").filter((x) => x.trim())) { process.stdout.write("security> "); status ||= run(l.trim().split(/\\s+/)) }
  process.exit(status)
} else process.exit(run(argv))
`,
    },
    { readStdin: ["security"] },
  )
}

/** Runs a copy of the script (`source`) with the fake security first on PATH. */
function keychain(source, cmd, dir, env = {}, fakes = fakeSecurity(fresh("fake"))) {
  const script = join(fresh("script"), "ci-keychain.sh")
  writeFileSync(script, source)
  const r = spawnSync("/bin/bash", [script, cmd, dir], {
    encoding: "utf8",
    env: {
      PATH: fakes.path(),
      GITHUB_ACTIONS: "true",
      APPLE_CERTIFICATE: P12,
      APPLE_CERTIFICATE_PASSWORD: PASSWORD,
      APPLE_SIGNING_IDENTITY: IDENTITY,
      ...env,
    },
  })
  return { ...r, fakes }
}

/** Every security command run, `-i` lines as their own: { argv, viaStdin }. */
const commands = (fakes) =>
  fakes.calls().flatMap((c) =>
    c.argv[0] === "-i"
      ? c.stdin
          .split("\n")
          .filter((l) => l.trim())
          .map((l) => ({ argv: l.trim().split(/\s+/), viaStdin: true }))
      : [{ argv: c.argv, viaStdin: false }],
  )
const searchList = (fakes) =>
  readFileSync(join(fakes.state, "list"), "utf8").split("\n").filter(Boolean)

/** The security commands that take a password. */
const WITH_PASSWORD = ["create-keychain", "unlock-keychain", "import", "set-key-partition-list"]

/** An import, then a delete, and what was wrong with how they went. */
function keychainProblems(source) {
  const p = []
  const dir = join(fresh("signing"), "signing")
  const imp = keychain(source, "import", dir)
  if (imp.status !== 0) return [`import failed: ${imp.stderr}`]
  const kc = join(dir, "ranch-signing.keychain-db")
  if (imp.stdout !== `keychain=${kc}\n`) p.push(`stdout: ${JSON.stringify(imp.stdout)}`)
  const cmds = commands(imp.fakes)
  for (const c of cmds) {
    if (!c.viaStdin && WITH_PASSWORD.includes(c.argv[0]))
      p.push(`a password on a command line: security ${c.argv[0]}`)
  }
  if (JSON.stringify(imp.fakes.calls()).includes(PASSWORD))
    p.push("the .p12 password reached security")
  const imports = cmds.filter((c) => c.argv[0] === "import")
  if (imports.length !== 1) p.push(`${imports.length} imports`)
  for (const c of imports) {
    if (!c.argv.includes("-x")) p.push("the key is importable without -x (exportable)")
    if (c.argv[c.argv.indexOf("-k") + 1] !== kc) p.push("imported into another keychain")
    if (c.argv[c.argv.indexOf("-T") + 1] !== "/usr/bin/codesign") p.push("not for codesign only")
  }
  if (JSON.stringify(searchList(imp.fakes)) !== JSON.stringify([kc, LOGIN]))
    p.push(`search list after import: ${searchList(imp.fakes)}`)
  if (cmds.some((c) => c.argv[0] === "default-keychain")) p.push("sets a default keychain")
  if (readdirSync(dir).some((f) => f.endsWith(".p12"))) p.push("a .p12 stays on disk")

  const del = keychain(source, "delete", dir, {}, imp.fakes)
  if (del.status !== 0) p.push(`delete failed: ${del.stderr}`)
  if (JSON.stringify(searchList(imp.fakes)) !== JSON.stringify([LOGIN]))
    p.push(`search list after delete: ${searchList(imp.fakes)}`)
  if (existsSync(dir) && readdirSync(dir).length) p.push(`files left: ${readdirSync(dir)}`)

  // A delete whose security fails still removes every file, and says so.
  const dir2 = join(fresh("signing"), "signing")
  const imp2 = keychain(source, "import", dir2)
  const failing = keychain(source, "delete", dir2, { FAKE_FAIL: "delete-keychain" }, imp2.fakes)
  if (failing.status === 0) p.push("a failed delete-keychain is not reported")
  if (existsSync(dir2) && readdirSync(dir2).length) p.push("a failed delete leaves files")
  return p
}

describe("ci-keychain.sh", () => {
  test("import then delete keep every rule", () => {
    assert.deepEqual(keychainProblems(SOURCE), [])
  })

  test("each rule bites: a copy of the script with it broken is caught", () => {
    const mutations = {
      "-x dropped": [" -x -P ", " -P "],
      "a password on argv": [
        'security_stdin "create-keychain -p $kc_pass $keychain"',
        'security create-keychain -p "$kc_pass" "$keychain" >&2',
      ],
      "the search list not put back": [
        // biome-ignore lint/suspicious/noTemplateCurlyInString: a line of the bash script
        '    security list-keychains -d user -s ${list[@]+"${list[@]}"} || status=$?\n',
        "",
      ],
      "files left when security fails": [
        '  if [ -d "$dir" ]; then find "$dir" -mindepth 1 -delete; fi\n',
        "",
      ],
      "the keychain not first": ['list=("$keychain")\n', "list=()\n"],
      "a default keychain set": [
        // biome-ignore lint/suspicious/noTemplateCurlyInString: a line of the bash script
        'security list-keychains -d user -s "${list[@]}" >&2\n',
        // biome-ignore lint/suspicious/noTemplateCurlyInString: a line of the bash script
        'security list-keychains -d user -s "${list[@]}" >&2\nsecurity default-keychain -s "$keychain"\n',
      ],
    }
    for (const [name, [from, to]] of Object.entries(mutations)) {
      assert.ok(SOURCE.includes(from), `${name}: the script changed, update the mutation`)
      assert.notDeepEqual(keychainProblems(SOURCE.replace(from, to)), [], `${name} went unnoticed`)
    }
  })

  test("refuses before any security call: outside Actions, a relative folder, a space, no certificate, a wrong password, another identity", () => {
    const cases = [
      [{ GITHUB_ACTIONS: "" }, "/abs/signing", /GitHub Actions only/],
      [{}, "rel/signing", /absolute path/],
      [{}, "/abs/sign ing", /no space/],
      [{ APPLE_CERTIFICATE: "" }, "/abs/signing", /APPLE_CERTIFICATE is empty/],
      [{ APPLE_SIGNING_IDENTITY: "" }, "/abs/signing", /APPLE_SIGNING_IDENTITY is empty/],
    ]
    for (const [env, dir, message] of cases) {
      const r = keychain(SOURCE, "import", dir, env)
      assert.equal(r.status, 1, JSON.stringify(env))
      assert.match(r.stderr, message)
      assert.deepEqual(r.fakes.calls(), [])
    }
    const wrong = keychain(SOURCE, "import", join(fresh("signing"), "signing"), {
      APPLE_CERTIFICATE_PASSWORD: "wrong",
    })
    assert.match(wrong.stderr, /could not be read with APPLE_CERTIFICATE_PASSWORD/)
    assert.deepEqual(wrong.fakes.calls(), [])
    const other = keychain(SOURCE, "import", join(fresh("signing"), "signing"), {
      FAKE_IDENTITY: "Apple Development: Someone",
    })
    assert.equal(other.status, 1)
    assert.match(other.stderr, /no valid code-signing identity named as APPLE_SIGNING_IDENTITY/)
  })

  test("delete is safe with nothing imported, and twice", () => {
    const dir = join(fresh("signing"), "signing")
    assert.equal(keychain(SOURCE, "delete", dir).status, 0)
    const fakes = fakeSecurity(fresh("fake"))
    assert.equal(keychain(SOURCE, "import", dir, {}, fakes).status, 0)
    assert.equal(keychain(SOURCE, "delete", dir, {}, fakes).status, 0)
    assert.equal(keychain(SOURCE, "delete", dir, {}, fakes).status, 0)
  })

  test("security is reached through PATH only (so the stand-in is the one that runs)", () => {
    assert.doesNotMatch(SOURCE, /\/usr\/bin\/security/)
  })
})
