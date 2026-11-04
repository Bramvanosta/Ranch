#!/usr/bin/env node
// Release pipelines: sign an update archive the way the Tauri updater checks
// it, with Node.js alone.
//
//   node scripts/minisign.mjs sign <file> --key-file <path> --version <version>
//   node scripts/minisign.mjs verify <file> --signature <file.sig>
//
// release.yml's signing jobs run it on a runner that installs no dependency:
// its imports are Node.js built-ins, nothing else (a test walks them). The
// private key is the TAURI_SIGNING_PRIVATE_KEY secret written to a 0600 file
// (`--key-file`), its password TAURI_SIGNING_PRIVATE_KEY_PASSWORD in the
// environment. It writes `<file>.sig` as `tauri signer sign` and the bundler
// do: the minisign signature's text, base64.
//
// The format (minisign, as rsign2 writes it, which is what the Tauri CLI uses):
//  - the key: base64 of "untrusted comment: …\n" and a base64 line of
//    "Ed" "Sc" "B2", a 32-byte scrypt salt, opslimit and memlimit (u64 LE),
//    then 104 bytes XORed with scrypt(password): key id (8), the Ed25519
//    secret key (seed 32, public key 32), a BLAKE2b-256 checksum (32);
//  - the signature: "ED" (the file is hashed with BLAKE2b-512 first), the key
//    id and the Ed25519 signature of that hash; then a trusted comment and the
//    signature of (signature || trusted comment).
// Node has no BLAKE2b-256, so the checksum is not read: the key is checked
// instead against the public key compiled into the app (tauri.conf.json's
// `plugins.updater.pubkey`). A wrong password gives another key, which fails
// that check, and every signature is verified against that public key before
// it is written.
import {
  createHash,
  createPrivateKey,
  createPublicKey,
  scryptSync,
  sign,
  verify,
} from "node:crypto"
import { readFileSync, writeFileSync } from "node:fs"
import { basename, dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

const REPO = join(dirname(fileURLToPath(import.meta.url)), "..")
const UNTRUSTED = "untrusted comment: signature from tauri secret key"
// DER prefixes of an Ed25519 key: PKCS#8 for the 32-byte seed, SPKI for the public key.
const PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex")
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex")

export class MinisignError extends Error {}

/** The second line of a minisign text block (the base64 payload), decoded. */
function payloadOf(text, what) {
  const lines = text.split("\n")
  if (!lines[0]?.startsWith("untrusted comment:") || !lines[1]) {
    throw new MinisignError(`${what} is not a minisign file`)
  }
  return Buffer.from(lines[1].trim(), "base64")
}

/** A base64 Tauri key (the secret's or tauri.conf.json's form) as its text. */
const unwrap = (b64) => Buffer.from(b64.trim(), "base64").toString("utf8")

/** `{ keyId, publicKey }` of tauri.conf.json's `pubkey` (or any base64 minisign public key). */
export function parsePublicKey(b64) {
  const raw = payloadOf(unwrap(b64), "the public key")
  if (raw.length !== 42 || raw.subarray(0, 2).toString() !== "Ed") {
    throw new MinisignError("the public key is not an Ed25519 minisign key")
  }
  return { keyId: raw.subarray(2, 10), publicKey: raw.subarray(10, 42) }
}

/** The public key the app trusts: tauri.conf.json's `plugins.updater.pubkey`. */
export function appPublicKey(conf = join(REPO, "src-tauri", "tauri.conf.json")) {
  return parsePublicKey(JSON.parse(readFileSync(conf, "utf8")).plugins.updater.pubkey)
}

/**
 * libsodium's crypto_pwhash_scryptsalsa208sha256 parameters for an opslimit
 * and a memlimit (its `pickparams`), as rsign2 derives the key's stream.
 */
export function scryptParams(opslimit, memlimit) {
  const ops = opslimit < 32768 ? 32768 : opslimit
  const r = 8
  let N_log2 = 1
  let p
  if (ops < memlimit / 32) {
    p = 1
    const maxN = ops / (r * 4)
    while (N_log2 < 63 && 2 ** N_log2 <= maxN / 2) N_log2++
  } else {
    const maxN = memlimit / (r * 128)
    while (N_log2 < 63 && 2 ** N_log2 <= maxN / 2) N_log2++
    const maxrp = Math.min(Math.floor(ops / 4 / 2 ** N_log2), 0x3fffffff)
    p = Math.floor(maxrp / r)
  }
  return { N: 2 ** N_log2, r, p }
}

/**
 * The secret key of a base64 Tauri key and its password: `{ keyId, seed,
 * publicKey }`, checked against `expected` (parsePublicKey's form).
 */
export function parseSecretKey(b64, password, expected) {
  const raw = payloadOf(unwrap(b64), "the private key")
  if (raw.length !== 158 || raw.subarray(0, 2).toString() !== "Ed") {
    throw new MinisignError("the private key is not an Ed25519 minisign secret key")
  }
  const kdf = raw.subarray(2, 4).toString("latin1")
  let keynum = raw.subarray(54, 158)
  if (kdf === "Sc") {
    const salt = raw.subarray(6, 38)
    const { N, r, p } = scryptParams(
      Number(raw.readBigUInt64LE(38)),
      Number(raw.readBigUInt64LE(46)),
    )
    const stream = scryptSync(Buffer.from(password ?? "", "utf8"), salt, 104, {
      N,
      r,
      p,
      maxmem: 256 * N * r + 1024 * 1024,
    })
    keynum = Buffer.from(keynum.map((b, i) => b ^ stream[i]))
  } else if (kdf !== "\0\0") {
    throw new MinisignError(
      `the private key uses an unknown key derivation (${JSON.stringify(kdf)})`,
    )
  }
  const key = {
    keyId: keynum.subarray(0, 8),
    seed: keynum.subarray(8, 40),
    publicKey: keynum.subarray(40, 72),
  }
  const derived = createPublicKey(privateKeyOf(key.seed))
    .export({ format: "der", type: "spki" })
    .subarray(12)
  if (!derived.equals(key.publicKey)) {
    throw new MinisignError(
      "the private key does not open with this password (TAURI_SIGNING_PRIVATE_KEY_PASSWORD)",
    )
  }
  if (expected && !(key.keyId.equals(expected.keyId) && key.publicKey.equals(expected.publicKey))) {
    throw new MinisignError(
      `the private key (id ${hexId(key.keyId)}) is not the one the app trusts (id ${hexId(expected.keyId)}, tauri.conf.json)`,
    )
  }
  return key
}

/** A key id as minisign prints it: the 8 bytes little-endian, in hex. */
const hexId = (id) => Buffer.from(id).reverse().toString("hex").toUpperCase()
const privateKeyOf = (seed) =>
  createPrivateKey({ key: Buffer.concat([PKCS8_PREFIX, seed]), format: "der", type: "pkcs8" })
const publicKeyOf = (pk) =>
  createPublicKey({ key: Buffer.concat([SPKI_PREFIX, pk]), format: "der", type: "spki" })

/** The trusted comment the Tauri bundler writes. */
export const trustedComment = (file, version, timestamp) =>
  `timestamp:${timestamp}\tfile:${basename(file)}\tversion:${version}`

/** The `.sig` content (base64 of the signature's text) of `data` with `key`. */
export function signData(data, key, comment) {
  const digest = createHash("blake2b512").update(data).digest()
  const signature = sign(null, digest, privateKeyOf(key.seed))
  const global = sign(
    null,
    Buffer.concat([signature, Buffer.from(comment, "utf8")]),
    privateKeyOf(key.seed),
  )
  const line = Buffer.concat([Buffer.from("ED"), key.keyId, signature]).toString("base64")
  const text = `${UNTRUSTED}\n${line}\ntrusted comment: ${comment}\n${global.toString("base64")}\n`
  return Buffer.from(text, "utf8").toString("base64")
}

/** Whether a `.sig` content signs `data` for `expected` (both signatures and the key id); throws otherwise. */
export function verifyData(data, sig, expected) {
  const lines = unwrap(sig).split("\n")
  const raw = payloadOf(lines.join("\n"), "the signature")
  const comment = lines[2]?.startsWith("trusted comment: ") ? lines[2].slice(17) : null
  if (raw.length !== 74 || comment === null || !lines[3])
    throw new MinisignError("the signature is malformed")
  const alg = raw.subarray(0, 2).toString()
  if (alg !== "ED" && alg !== "Ed") throw new MinisignError(`unknown signature algorithm ${alg}`)
  if (!raw.subarray(2, 10).equals(expected.keyId)) {
    throw new MinisignError(
      `signed by key ${hexId(raw.subarray(2, 10))}, not ${hexId(expected.keyId)}`,
    )
  }
  const pk = publicKeyOf(expected.publicKey)
  const signed = alg === "ED" ? createHash("blake2b512").update(data).digest() : data
  if (!verify(null, signed, pk, raw.subarray(10)))
    throw new MinisignError("the signature does not match the file")
  const global = Buffer.from(lines[3].trim(), "base64")
  if (!verify(null, Buffer.concat([raw.subarray(10), Buffer.from(comment, "utf8")]), pk, global)) {
    throw new MinisignError("the trusted comment's signature does not match")
  }
  return { comment }
}

/** `sign`: `<file>.sig` written, after its check against the app's public key. Returns its path. */
export function signFile(
  file,
  { keyFile, version, env = process.env, expected = appPublicKey(), now = Date.now() },
) {
  const key = parseSecretKey(
    readFileSync(keyFile, "utf8"),
    env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD,
    expected,
  )
  const data = readFileSync(file)
  const sig = signData(data, key, trustedComment(file, version, Math.floor(now / 1000)))
  verifyData(data, sig, expected)
  writeFileSync(`${file}.sig`, sig)
  return `${file}.sig`
}

export function parseArgs(argv) {
  const [command, file, ...rest] = argv
  const flags = { "--key-file": "keyFile", "--version": "version", "--signature": "signature" }
  const out = { command, file }
  for (let i = 0; i < rest.length; i += 2) {
    const name = flags[rest[i]]
    if (!name || rest[i + 1] === undefined) throw new MinisignError(`unknown argument "${rest[i]}"`)
    out[name] = rest[i + 1]
  }
  const need = { sign: ["keyFile", "version"], verify: ["signature"] }[command]
  if (!need || !file) {
    throw new MinisignError(
      "usage: minisign.mjs sign <file> --key-file <path> --version <version> | verify <file> --signature <file.sig>",
    )
  }
  for (const k of need)
    if (!out[k])
      throw new MinisignError(
        `${command} needs --${k.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`,
      )
  return out
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    const args = parseArgs(process.argv.slice(2))
    if (args.command === "sign") {
      const sig = signFile(args.file, args)
      console.log(`minisign: ${basename(sig)} written and verified against tauri.conf.json's key`)
    } else {
      const { comment } = verifyData(
        readFileSync(args.file),
        readFileSync(args.signature, "utf8"),
        appPublicKey(),
      )
      console.log(`minisign: ${basename(args.file)} verified (${comment})`)
    }
  } catch (e) {
    console.error(`minisign: ${e instanceof MinisignError ? e.message : (e?.stack ?? e)}`)
    process.exit(1)
  }
}
