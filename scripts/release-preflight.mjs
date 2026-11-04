#!/usr/bin/env node
// Release pipelines: what the `release` environment holds, before anything is
// built. Each set of secrets is all or nothing: none and the release is built
// without it (said); all and it is used; some and the run fails here, naming
// what is missing, rather than an hour later in a signing job.
//
//   node scripts/release-preflight.mjs --output "$GITHUB_OUTPUT"
//
// release.yml's `release` job runs it with each secret's presence only
// (`HAS_<NAME>: ${{ secrets.<NAME> != '' }}`), never a value. It prints
// `signed=true|false`, `updates=on|off` and `tap=on|off` for the jobs after it.

import { appendFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

export const SETS = {
  apple: {
    what: "the Developer ID signature and notarization of the macOS app",
    without:
      "the macOS app is signed ad hoc and not notarized: Gatekeeper refuses it once downloaded",
    names: [
      "APPLE_CERTIFICATE",
      "APPLE_CERTIFICATE_PASSWORD",
      "APPLE_SIGNING_IDENTITY",
      "APPLE_TEAM_ID",
      "APPLE_API_KEY",
      "APPLE_API_ISSUER",
      "APPLE_API_KEY_P8",
    ],
    output: ["signed", "true", "false"],
  },
  updater: {
    what: "the updater archives' signatures",
    without: "no updater archive is signed and no update is published",
    names: ["TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"],
    output: ["updates", "on", "off"],
  },
  tap: {
    what: "the Homebrew casks",
    without: "the Homebrew tap is not updated",
    names: ["HOMEBREW_TAP_DEPLOY_KEY"],
    output: ["tap", "on", "off"],
  },
}
/** Names an earlier workflow read, now unused: said, so they get deleted. */
export const RETIRED = ["APPLE_ID", "APPLE_PASSWORD"]

/** `{ outputs, warnings, errors }` of the presences in `env` (HAS_<NAME> = "true" | "false"). */
export function preflight(env) {
  const outputs = {}
  const warnings = []
  const errors = []
  for (const set of Object.values(SETS)) {
    const missing = set.names.filter((n) => env[`HAS_${n}`] !== "true")
    const [key, on, off] = set.output
    if (missing.length === 0) outputs[key] = on
    else if (missing.length === set.names.length) {
      outputs[key] = off
      warnings.push(`No ${set.names.join(", ")}: ${set.without}`)
    } else {
      errors.push(
        `${set.what} needs all of ${set.names.join(", ")}, or none: missing ${missing.join(", ")}`,
      )
    }
  }
  for (const n of RETIRED) {
    if (env[`HAS_${n}`] === "true")
      warnings.push(`${n} is no longer used (notarization takes the API key): delete it`)
  }
  return { outputs, warnings, errors }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const at = process.argv.indexOf("--output")
  const file = at > 0 ? process.argv[at + 1] : undefined
  if (!file) {
    console.error("usage: release-preflight.mjs --output <file>")
    process.exit(1)
  }
  const { outputs, warnings, errors } = preflight(process.env)
  for (const w of warnings) console.log(`::warning::${w}`)
  for (const e of errors) console.log(`::error::${e}`)
  if (errors.length) process.exit(1)
  const lines = Object.entries(outputs).map(([k, v]) => `${k}=${v}\n`)
  appendFileSync(file, lines.join(""))
  process.stdout.write(lines.join(""))
}
