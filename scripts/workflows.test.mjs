// node --test scripts/   (run by `pnpm test`)
//
// Every workflow of .github/workflows and the pnpm composite action, held to
// the rules that are not release.yml's own (release-workflow.test.mjs has
// those): actions pinned to exact releases, checkouts without credentials, no
// `${{ }}` inside a script, write permissions on jobs only, no secret outside
// release.yml, one Rust everywhere. Each rule comes with a mutation of the
// files it must catch.
import assert from "node:assert/strict"
import { readdirSync, readFileSync } from "node:fs"
import { join } from "node:path"
import { describe, test } from "node:test"
import YAML from "yaml"

import { PINS } from "./fixtures/action-pins.mjs"

const REPO = join(import.meta.dirname, "..")
const WORKFLOWS = join(REPO, ".github", "workflows")
const COMPOSITE = ".github/actions/pnpm/action.yml"
const read = (p) => readFileSync(join(REPO, p), "utf8")
/** The files the rules read, by path: every workflow, the composite action, the Rust pins. */
const FILES = Object.fromEntries(
  [
    ...readdirSync(WORKFLOWS)
      .filter((f) => f.endsWith(".yml"))
      .sort()
      .map((f) => `.github/workflows/${f}`),
    COMPOSITE,
    "rust-toolchain.toml",
    "mise.toml",
    "Dockerfile",
  ].map((p) => [p, read(p)]),
)
const workflowsOf = (files) =>
  Object.entries(files)
    .filter(([p]) => p.startsWith(".github/workflows/"))
    .map(([p, text]) => ({ path: p, text, wf: YAML.parse(text) }))
const stepsOf = (files) => [
  ...workflowsOf(files).flatMap(({ path, wf }) =>
    Object.entries(wf.jobs).flatMap(([job, j]) =>
      (j.steps ?? []).map((s) => ({ where: `${path} › ${job}`, ...s })),
    ),
  ),
  ...YAML.parse(files[COMPOSITE]).runs.steps.map((s) => ({ where: COMPOSITE, ...s })),
]
const expr = (e) => `\${{ ${e} }}`

const RULES = {
  "every action is pinned to the commit of an exact release, the tag beside it, and is a known one":
    (files) => {
      const p = []
      const used = new Set()
      for (const [path, text] of Object.entries(files).filter(([f]) => f.endsWith(".yml"))) {
        for (const line of text.split("\n").filter((l) => /^\s*-?\s*uses:/.test(l))) {
          if (/uses:\s*\.\/\.github\/actions\/[\w-]+\s*$/.test(line)) continue
          const m = /uses:\s*([\w.-]+\/[\w.-]+)@([0-9a-f]{40})\s+#\s+(v\d+\.\d+\.\d+)\s*$/.exec(
            line,
          )
          if (!m) p.push(`${path}: not pinned to an exact release: ${line.trim()}`)
          else if (JSON.stringify(PINS[m[1]]) !== JSON.stringify([m[2], m[3]]))
            p.push(`${path}: unexpected pin: ${line.trim()}`)
          else used.add(m[1])
        }
      }
      for (const a of Object.keys(PINS)) if (!used.has(a)) p.push(`${a} is pinned but used nowhere`)
      return p
    },

  "every checkout keeps no credential": (files) =>
    stepsOf(files)
      .filter(
        (s) => s.uses?.startsWith("actions/checkout@") && s.with?.["persist-credentials"] !== false,
      )
      .map((s) => `${s.where}: persist-credentials`),

  "no ${{ }} inside a script": (files) =>
    stepsOf(files)
      .filter((s) => s.run?.includes(expr("").slice(0, 3)))
      .map((s) => `${s.where} › ${s.name ?? s.id}: \${{ in run`),

  "the workflow reads by default; a write permission belongs to the job that needs it": (files) =>
    workflowsOf(files).flatMap(({ path, wf }) => {
      const p = []
      if (JSON.stringify(wf.permissions) !== JSON.stringify({ contents: "read" }))
        p.push(`${path}: ${JSON.stringify(wf.permissions)}`)
      return p
    }),

  "no secret outside release.yml, and the job token as github.token": (files) =>
    workflowsOf(files).flatMap(({ path, text }) => {
      const p = []
      if (path !== ".github/workflows/release.yml" && /secrets\./.test(text))
        p.push(`${path} reads a secret`)
      if (/secrets\.GITHUB_TOKEN/.test(text)) p.push(`${path}: secrets.GITHUB_TOKEN`)
      return p
    }),

  "one Rust: rust-toolchain.toml's version in every workflow, mise.toml and the Dockerfile": (
    files,
  ) => {
    const p = []
    const channel = /^channel = "([\d.]+)"$/m.exec(files["rust-toolchain.toml"])?.[1]
    if (!channel) return ["rust-toolchain.toml names no channel"]
    const mise = /^rust = "([\d.]+)"/m.exec(files["mise.toml"])?.[1]
    const docker = /^FROM rust:([\d.]+)-/m.exec(files.Dockerfile)?.[1]
    if (mise !== channel) p.push(`mise.toml: ${mise}`)
    if (docker !== channel) p.push(`Dockerfile: ${docker}`)
    for (const { path, text } of workflowsOf(files)) {
      for (const [, v] of text.matchAll(/rustup toolchain install ([\d.]+)/g))
        if (v !== channel) p.push(`${path}: rustup ${v}`)
      for (const [, v] of text.matchAll(/RUSTUP_TOOLCHAIN=([\d.]+)/g))
        if (v !== channel) p.push(`${path}: RUSTUP_TOOLCHAIN ${v}`)
      const installs = (text.match(/rustup toolchain install /g) ?? []).length
      if (installs !== (text.match(/RUSTUP_TOOLCHAIN=/g) ?? []).length)
        p.push(`${path}: a toolchain installed but not selected`)
      if (/dtolnay\/rust-toolchain|toolchain:\s/.test(text)) p.push(`${path}: a toolchain action`)
    }
    return p
  },

  "npm installs exactly a lockfile (npm ci), never npm install": (files) =>
    stepsOf(files)
      .filter((s) => /\bnpm (?:install|i)\b/.test(s.run ?? ""))
      .map((s) => `${s.where}: npm install`),
}

const mutate = (path, from, to) => (files) => {
  assert.ok(
    files[path].includes(from),
    `${path}: "${from}" is not there any more, update the mutation`,
  )
  files[path] = files[path].replace(from, to)
}
const MUTATIONS = {
  "every action is pinned to the commit of an exact release, the tag beside it, and is a known one":
    mutate(
      ".github/workflows/image.yml",
      "c94ce9fb468520275223c153574b00df6fe4bcc9 # v3.7.0",
      "c94ce9fb468520275223c153574b00df6fe4bcc9 # v3",
    ),
  "every checkout keeps no credential": mutate(
    ".github/workflows/ci.yml",
    "persist-credentials: false",
    "persist-credentials: true",
  ),
  "no ${{ }} inside a script": mutate(
    ".github/workflows/ci.yml",
    "run: bash scripts/check-no-creds-in-sql.sh",
    `run: echo "${expr("github.head_ref")}"`,
  ),
  "the workflow reads by default; a write permission belongs to the job that needs it": mutate(
    ".github/workflows/image.yml",
    "permissions:\n  contents: read\n",
    "permissions:\n  contents: read\n  packages: write\n",
  ),
  "no secret outside release.yml, and the job token as github.token": mutate(
    ".github/workflows/image.yml",
    `password: ${expr("github.token")}`,
    `password: ${expr("secrets.GITHUB_TOKEN")}`,
  ),
  "one Rust: rust-toolchain.toml's version in every workflow, mise.toml and the Dockerfile": mutate(
    ".github/workflows/ci.yml",
    "rustup toolchain install 1.99.1 --profile minimal --component clippy",
    "rustup toolchain install 1.98.0 --profile minimal --component clippy",
  ),
  "npm installs exactly a lockfile (npm ci), never npm install": mutate(
    ".github/workflows/ci.yml",
    "(cd .docker/bullmq-seed && npm ci",
    "(cd .docker/bullmq-seed && npm install",
  ),
}

describe("every workflow: its rules, each caught by its mutation", () => {
  for (const [name, rule] of Object.entries(RULES)) {
    test(name, () => {
      assert.deepEqual(rule(FILES), [])
      const files = { ...FILES }
      MUTATIONS[name](files)
      assert.notDeepEqual(rule(files), [], `the mutation of "${name}" went unnoticed`)
    })
  }

  test("every rule has its mutation, and only rules have one", () => {
    assert.deepEqual(Object.keys(MUTATIONS).sort(), Object.keys(RULES).sort())
  })

  test("the rules read every workflow there is", () => {
    assert.deepEqual(
      workflowsOf(FILES).map((w) => w.path),
      [".github/workflows/ci.yml", ".github/workflows/image.yml", ".github/workflows/release.yml"],
    )
  })

  test("more Rust mutations: mise.toml, the Dockerfile, a toolchain action, a toolchain installed but not selected", () => {
    const rule =
      RULES[
        "one Rust: rust-toolchain.toml's version in every workflow, mise.toml and the Dockerfile"
      ]
    const cases = [
      mutate("mise.toml", 'rust = "1.99.1"', 'rust = "1.98.0"'),
      mutate("Dockerfile", "FROM rust:1.99.1-", "FROM rust:1.98.0-"),
      mutate(
        ".github/workflows/ci.yml",
        "      - name: Rust 1.99.1\n",
        "      - uses: dtolnay/rust-toolchain@stable\n      - name: Rust 1.99.1\n",
      ),
      mutate(
        ".github/workflows/release.yml",
        '          echo "RUSTUP_TOOLCHAIN=1.99.1" >> "$GITHUB_ENV"\n',
        "",
      ),
    ]
    for (const m of cases) {
      const files = { ...FILES }
      m(files)
      assert.notDeepEqual(rule(files), [])
    }
  })
})
