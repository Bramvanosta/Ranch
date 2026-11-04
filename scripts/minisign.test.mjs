// node --test scripts/   (run by `pnpm test`)
//
// fixtures/minisign/ holds a throwaway key pair made for these tests only by
// the Tauri CLI the lockfile pins (2.12.4: `tauri signer generate -p
// test-password`), and that CLI's signature of payload.bin (`tauri signer
// sign`). The CLI's signatures are randomized, so ours are checked by
// verifying, not by comparing bytes.
import assert from "node:assert/strict"
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { after, test } from "node:test"

import {
  appPublicKey,
  MinisignError,
  parseArgs,
  parsePublicKey,
  parseSecretKey,
  scryptParams,
  signData,
  signFile,
  trustedComment,
  verifyData,
} from "./minisign.mjs"

const F = join(import.meta.dirname, "fixtures", "minisign")
const fixture = (name, enc) => readFileSync(join(F, name), enc)
const pub = parsePublicKey(fixture("test-only.key.pub", "utf8"))
const KEY = fixture("test-only.key", "utf8")
const PASSWORD = "test-password"
const tmp = mkdtempSync(join(tmpdir(), "ranch-minisign-"))
after(() => rmSync(tmp, { recursive: true, force: true }))

/** The `.sig` content with one line of its text replaced. */
const editLine = (sig, i, change) => {
  const lines = Buffer.from(sig, "base64").toString("utf8").split("\n")
  lines[i] = change(lines[i])
  return Buffer.from(lines.join("\n"), "utf8").toString("base64")
}

test("scrypt parameters are libsodium's pickparams (rsign2's key, and the sensitive limits)", () => {
  assert.deepEqual(scryptParams(1048576, 33554432), { N: 32768, r: 8, p: 1 })
  assert.deepEqual(scryptParams(33554432, 1073741824), { N: 1048576, r: 8, p: 1 })
  assert.deepEqual(scryptParams(1000, 1 << 30), { N: 1024, r: 8, p: 1 }) // opslimit floor 32768
})

test("the Tauri CLI's key opens with its password, and with no other", () => {
  const key = parseSecretKey(KEY, PASSWORD, pub)
  assert.deepEqual(key.keyId, pub.keyId)
  assert.deepEqual(key.publicKey, pub.publicKey)
  assert.throws(() => parseSecretKey(KEY, "wrong", pub), /does not open with this password/)
  assert.throws(() => parseSecretKey(KEY, undefined, pub), /does not open with this password/)
})

test("a key other than the app's is refused, naming both ids", () => {
  assert.throws(
    () => parseSecretKey(KEY, PASSWORD, appPublicKey()),
    /\(id 7982FE1FE5F332F6\) is not the one the app trusts \(id 5FA72BDF92FC7F23, tauri\.conf\.json\)/,
  )
})

test("the Tauri CLI's signature verifies", () => {
  const { comment } = verifyData(fixture("payload.bin"), fixture("payload.bin.sig", "utf8"), pub)
  assert.match(comment, /^timestamp:\d+\tfile:payload\.bin$/)
})

test("our signature: the bundler's layout, prehashed, verified", () => {
  const key = parseSecretKey(KEY, PASSWORD, pub)
  const data = Buffer.from("an update archive")
  const sig = signData(data, key, trustedComment("/x/Ranch.app.tar.gz", "0.5.0", 1793773210))
  const text = Buffer.from(sig, "base64").toString("utf8")
  assert.match(
    text,
    /^untrusted comment: signature from tauri secret key\nRUT2MvPlH\/6Ce[A-Za-z0-9+/=]+\ntrusted comment: timestamp:1793773210\tfile:Ranch\.app\.tar\.gz\tversion:0\.5\.0\n[A-Za-z0-9+/=]{88}\n$/,
  )
  assert.equal(
    verifyData(data, sig, pub).comment,
    "timestamp:1793773210\tfile:Ranch.app.tar.gz\tversion:0.5.0",
  )
})

test("verification refuses another file, an edited comment, another key id, another algorithm", () => {
  const sig = fixture("payload.bin.sig", "utf8")
  const data = fixture("payload.bin")
  assert.throws(() => verifyData(Buffer.from("other"), sig, pub), /does not match the file/)
  assert.throws(
    () =>
      verifyData(
        data,
        editLine(sig, 2, (l) => `${l}x`),
        pub,
      ),
    /trusted comment's signature/,
  )
  assert.throws(
    () => verifyData(data, sig, appPublicKey()),
    /signed by key 7982FE1FE5F332F6, not 5FA72BDF92FC7F23/,
  )
  const otherAlg = editLine(sig, 1, (l) => {
    const raw = Buffer.from(l, "base64")
    raw.write("XX", 0)
    return raw.toString("base64")
  })
  assert.throws(() => verifyData(data, otherAlg, pub), /unknown signature algorithm XX/)
  assert.throws(
    () => verifyData(data, Buffer.from("nothing").toString("base64"), pub),
    MinisignError,
  )
})

test("signFile writes <file>.sig from a key file, with the password from the environment", () => {
  const file = join(tmp, "Ranch_0.5.0_amd64.AppImage")
  writeFileSync(file, "an AppImage")
  const keyFile = join(tmp, "updater.key")
  writeFileSync(keyFile, `${KEY}\n`, { mode: 0o600 })
  const env = { TAURI_SIGNING_PRIVATE_KEY_PASSWORD: PASSWORD }
  const sig = signFile(file, { keyFile, version: "0.5.0", env, expected: pub, now: 1793773210123 })
  assert.equal(sig, `${file}.sig`)
  assert.equal(statSync(keyFile).mode & 0o777, 0o600)
  const { comment } = verifyData(readFileSync(file), readFileSync(sig, "utf8"), pub)
  assert.equal(comment, "timestamp:1793773210\tfile:Ranch_0.5.0_amd64.AppImage\tversion:0.5.0")
  assert.throws(
    () => signFile(file, { keyFile, version: "0.5.0", env: {}, expected: pub }),
    /password/,
  )
})

test("the command line: sign needs --key-file and --version, verify --signature", () => {
  assert.deepEqual(parseArgs(["sign", "a", "--key-file", "k", "--version", "1.0.0"]), {
    command: "sign",
    file: "a",
    keyFile: "k",
    version: "1.0.0",
  })
  assert.throws(() => parseArgs(["sign", "a", "--key-file", "k"]), /sign needs --version/)
  assert.throws(() => parseArgs(["verify", "a"]), /verify needs --signature/)
  assert.throws(() => parseArgs(["sign", "a", "--key", "k"]), /unknown argument "--key"/)
  assert.throws(() => parseArgs(["publish", "a"]), /usage/)
})
