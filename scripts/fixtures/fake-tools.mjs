// Stand-ins for the system's tools (security, codesign, xcrun, hdiutil, gh…),
// for the release scripts' tests: a folder put first on PATH, each tool a Node
// script that records its arguments (and its standard input, when asked) in
// calls.jsonl, then runs the body it was given. The real ones never run.
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"

/**
 * `tools`: { name: body } where `body` is JavaScript run with `argv` (the
 * arguments), `fs`, `path`, `env` (process.env), `state` (a folder of the
 * fakes' own) and `stdin` (when `readStdin` names the tool) in scope.
 */
export function fakeBin(dir, tools, { readStdin = [] } = {}) {
  const bin = join(dir, "bin")
  const state = join(dir, "state")
  mkdirSync(bin, { recursive: true })
  mkdirSync(state, { recursive: true })
  const log = join(dir, "calls.jsonl")
  for (const [name, body] of Object.entries(tools)) {
    const file = join(bin, name)
    writeFileSync(
      file,
      `#!${process.execPath}
const fs = require("node:fs"), path = require("node:path")
const argv = process.argv.slice(2), env = process.env, state = ${JSON.stringify(state)}
const stdin = ${readStdin.includes(name) ? 'fs.readFileSync(0, "utf8")' : "null"}
fs.appendFileSync(${JSON.stringify(log)}, JSON.stringify({ tool: ${JSON.stringify(name)}, argv, stdin }) + "\\n")
${body}
`,
    )
    chmodSync(file, 0o755)
  }
  const calls = () =>
    existsSync(log)
      ? readFileSync(log, "utf8")
          .split("\n")
          .filter(Boolean)
          .map((l) => JSON.parse(l))
      : []
  return { bin, state, calls, path: (rest = "/usr/bin:/bin") => `${bin}:${rest}` }
}
