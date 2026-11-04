// node --test scripts/   (run by `pnpm test`)
//
// .github/workflows/release.yml read as data and held to the rules of
// docs/releasing.md ("What the workflow trusts"): no secret where dependency
// code runs, each secret only in the step that uses it, the `release`
// environment on the jobs that sign and publish, actions pinned to exact
// releases, signing jobs that run this repository's scripts only. Each rule
// is a function of the parsed workflow; each comes with a mutation of the
// workflow it must catch, so a rule that stops biting fails here too.
// Then the step scripts themselves, run in bash against stand-ins (gh,
// security, node) — never the real keychain or GitHub.
import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs"
import { tmpdir } from "node:os"
import { dirname, join, relative, resolve } from "node:path"
import { after, describe, test } from "node:test"
import YAML from "yaml"

import { PINS } from "./fixtures/action-pins.mjs"
import { fakeBin } from "./fixtures/fake-tools.mjs"
import { SETS } from "./release-preflight.mjs"

const REPO = join(import.meta.dirname, "..")
const raw = readFileSync(join(REPO, ".github", "workflows", "release.yml"), "utf8")
const composite = readFileSync(join(REPO, ".github", "actions", "pnpm", "action.yml"), "utf8")
const WF = YAML.parse(raw)
const tmp = mkdtempSync(join(tmpdir(), "ranch-release-workflow-"))
after(() => rmSync(tmp, { recursive: true, force: true }))
let n = 0
const fresh = (name) => {
  const d = join(tmp, `${name}-${n++}`)
  mkdirSync(d, { recursive: true })
  return d
}

const BUILD_JOBS = ["build-linux", "build-macos"]
const SIGN_JOBS = ["sign-linux", "sign-macos"]
const sha256 = (data) => createHash("sha256").update(data).digest("hex")
/** A GitHub expression as the workflow writes it: `${{ <e> }}`. */
const expr = (e) => `\${{ ${e} }}`

/** The only steps that may hold a secret's value, and which ones. */
const SECRET_STEPS = {
  "sign-macos › Signing material": [
    "APPLE_CERTIFICATE",
    "APPLE_CERTIFICATE_PASSWORD",
    "APPLE_SIGNING_IDENTITY",
    "APPLE_API_KEY_P8",
  ],
  "sign-macos › Sign, notarize, the DMG": [
    "APPLE_SIGNING_IDENTITY",
    "APPLE_TEAM_ID",
    "APPLE_API_KEY",
    "APPLE_API_ISSUER",
  ],
  "sign-macos › Sign the update archive": [
    "TAURI_SIGNING_PRIVATE_KEY",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
  ],
  "sign-linux › Sign the AppImage": [
    "TAURI_SIGNING_PRIVATE_KEY",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
  ],
  "publish › Update the Homebrew tap": ["HOMEBREW_TAP_DEPLOY_KEY"],
}
/** What a signing job may run: these scripts, nothing installed. */
const SIGN_NODE = ["scripts/minisign.mjs", "scripts/sign-macos.mjs"]
const SIGN_BASH = ["scripts/ci-keychain.sh"]
/** Build or dependency code: what only the build jobs may run. */
const BUILD_TOOL =
  /(?:^|[\s;|&(])(?:pnpm|npm|npx|corepack|cargo|rustup|rustc|brew|tauri|lipo|apt-get)\s/m
const CALLS_GH = /(?:^|[\s;|&($])gh\s/m
const SECRET_VALUE = /^\$\{\{\s*secrets\.([A-Z0-9_]+)\s*\}\}$/
const SECRET_PRESENCE = /^\$\{\{ secrets\.([A-Z0-9_]+) != '' \}\}$/
const usesToken = (s) =>
  Object.values(s.env ?? {}).some((v) => /github\.token|secrets\.GITHUB_TOKEN/.test(String(v)))

const jobsOf = (wf) => wf.jobs
const stepsOf = (wf) =>
  Object.entries(wf.jobs).flatMap(([job, j]) => (j.steps ?? []).map((s) => ({ job, ...s })))
const label = (s) => `${s.job} › ${s.name ?? s.uses ?? s.id}`
const secretsOf = (s) =>
  Object.fromEntries(
    Object.entries(s.env ?? {}).flatMap(([k, v]) => {
      const m = SECRET_VALUE.exec(String(v))
      return m ? [[k, m[1]]] : []
    }),
  )
const step = (wf, job, name) => {
  const s = wf.jobs[job].steps.find((x) => x.name === name)
  if (!s) throw new Error(`no step "${name}" in ${job}`)
  return s
}
/** A step's script and the repository scripts it runs, as one text. */
const scriptOf = (s) =>
  [
    s.run ?? "",
    ...[...(s.run ?? "").matchAll(/(?:bash|node) (scripts\/[\w.-]+\.(?:sh|mjs))/g)].map((m) =>
      readFileSync(join(REPO, m[1]), "utf8"),
    ),
  ].join("\n")
const usesLines = (text) => text.split("\n").filter((l) => /^\s*-?\s*uses:/.test(l))

/** The rules: each returns its problems (none: the workflow keeps it). */
const RULES = {
  "a pushed v* tag is the only trigger": (wf) =>
    JSON.stringify(wf.on) === JSON.stringify({ push: { tags: ["v*"] } })
      ? []
      : [`on: ${JSON.stringify(wf.on)}`],

  "reads by default; contents: write only on release, attach, publish": (wf) => {
    const p = []
    if (JSON.stringify(wf.permissions) !== JSON.stringify({ contents: "read" }))
      p.push("workflow permissions")
    for (const [name, j] of Object.entries(jobsOf(wf))) {
      const writes = ["release", "attach", "publish"].includes(name)
      if (!j.permissions) p.push(`${name}: no permissions`)
      else if ((j.permissions.contents === "write") !== writes)
        p.push(`${name}: contents ${j.permissions.contents}`)
      if (
        [...BUILD_JOBS, ...SIGN_JOBS].includes(name) &&
        JSON.stringify(j.permissions) !== '{"contents":"read"}'
      ) {
        p.push(`${name}: permissions ${JSON.stringify(j.permissions)}`)
      }
    }
    return p
  },

  "a build job holds no secret, no variable, no token, no environment": (wf) =>
    BUILD_JOBS.flatMap((name) => {
      const j = jobsOf(wf)[name]
      const p = []
      if (!j) return [`no job ${name}`]
      if (/secrets\.|vars\.|github\.token/.test(JSON.stringify(j)))
        p.push(`${name} reads a secret, a variable or the token`)
      if (j.environment) p.push(`${name} has an environment`)
      return p
    }),

  "environment: release on release, the signing jobs and publish, nowhere else": (wf) => {
    const want = ["release", "sign-linux", "sign-macos", "publish"]
    const got = Object.entries(jobsOf(wf))
      .filter(([, j]) => j.environment)
      .map(([k, j]) => `${k}:${j.environment}`)
    return JSON.stringify(got) === JSON.stringify(want.map((k) => `${k}:release`))
      ? []
      : [`environments: ${got}`]
  },

  "secrets only in a step's env, by value or presence; the release job sees presence only": (
    wf,
  ) => {
    const p = []
    if (/secrets\./.test(JSON.stringify(wf.env ?? {}))) p.push("workflow env")
    for (const [name, j] of Object.entries(jobsOf(wf))) {
      if (/secrets\./.test(JSON.stringify(j.env ?? {}))) p.push(`${name}: job env`)
      if (/secrets\./.test(JSON.stringify(j.outputs ?? {}))) p.push(`${name}: outputs`)
    }
    for (const s of stepsOf(wf)) {
      if (
        /secrets\./.test(JSON.stringify({ with: s.with ?? {}, if: s.if ?? "", run: s.run ?? "" }))
      )
        p.push(`${label(s)}: outside env`)
      for (const [k, v] of Object.entries(s.env ?? {})) {
        if (!/secrets\./.test(String(v))) continue
        const presence = SECRET_PRESENCE.test(String(v))
        if (!presence && !SECRET_VALUE.test(String(v))) p.push(`${label(s)} ${k}: ${v}`)
        if (s.job === "release" && !presence) p.push(`${label(s)} ${k}: a value in the release job`)
      }
    }
    if (/secrets\.GITHUB_TOKEN|toJSON\(secrets\)/.test(JSON.stringify(wf)))
      p.push("secrets.GITHUB_TOKEN or toJSON(secrets)")
    return p
  },

  "each secret's value only in the step that uses it": (wf) => {
    const p = []
    const got = {}
    for (const s of stepsOf(wf)) {
      const bound = secretsOf(s)
      if (Object.keys(bound).length) got[label(s)] = Object.values(bound).sort()
      for (const k of Object.keys(bound))
        if (!scriptOf(s).includes(k)) p.push(`${label(s)} binds ${k} and never reads it`)
    }
    const want = Object.fromEntries(
      Object.entries(SECRET_STEPS).map(([k, v]) => [k, [...v].sort()]),
    )
    if (
      JSON.stringify(got, Object.keys(got).sort()) !==
      JSON.stringify(want, Object.keys(want).sort())
    ) {
      p.push(`secrets by step: ${JSON.stringify(got)}`)
    }
    return p
  },

  "every action pinned to an exact release's commit, the tag beside it; the local action in build jobs only":
    (wf, texts = [raw, composite]) => {
      const p = []
      for (const line of texts.flatMap(usesLines)) {
        if (/uses:\s*\.\/\.github\/actions\/pnpm\s*$/.test(line)) continue
        const m = /uses:\s*([\w.-]+\/[\w.-]+)@([0-9a-f]{40})\s+#\s+(v\d+\.\d+\.\d+)\s*$/.exec(line)
        if (!m) p.push(`not pinned to an exact release: ${line.trim()}`)
        else if (JSON.stringify(PINS[m[1]]) !== JSON.stringify([m[2], m[3]]))
          p.push(`unexpected pin: ${line.trim()}`)
      }
      for (const s of stepsOf(wf))
        if (s.uses?.startsWith("./") && !BUILD_JOBS.includes(s.job))
          p.push(`${label(s)}: local action`)
      return p
    },

  "every job checks out with persist-credentials: false": (wf) =>
    Object.entries(jobsOf(wf)).flatMap(([name, j]) => {
      const c = j.steps.find((s) => s.uses?.startsWith("actions/checkout@"))
      return c?.with?.["persist-credentials"] === false
        ? []
        : [`${name}: checkout keeps credentials`]
    }),

  "no ${{ }} inside a script: values come through env": (wf) =>
    [...stepsOf(wf).filter((s) => s.run), ...YAML.parse(composite).runs.steps.filter((s) => s.run)]
      .filter((s) => s.run.includes(expr("").slice(0, 3)))
      .map((s) => `${s.job ? label(s) : "pnpm action"}: \${{ in run`),

  "the job token only in steps that call gh, never beside a signing secret": (wf) => {
    const p = []
    for (const s of stepsOf(wf)) {
      const gh = CALLS_GH.test(scriptOf(s))
      if (usesToken(s) !== gh) p.push(`${label(s)}: token ${usesToken(s)}, calls gh ${gh}`)
      const signing = Object.values(secretsOf(s)).filter((x) => /^(?:APPLE|TAURI)_/.test(x))
      if (usesToken(s) && signing.length) p.push(`${label(s)}: token beside ${signing}`)
      if (usesToken(s) && [...BUILD_JOBS, ...SIGN_JOBS].includes(s.job))
        p.push(`${label(s)}: token in ${s.job}`)
    }
    return p
  },

  "no build tool outside the build jobs": (wf) =>
    stepsOf(wf)
      .filter((s) => !BUILD_JOBS.includes(s.job) && BUILD_TOOL.test(s.run ?? ""))
      .map((s) => `${label(s)} runs a build tool`),

  "a signing job uses GitHub's checkout, Node and artifact actions, and runs this repository's signing scripts only":
    (wf) => {
      const p = []
      const allowed = [
        "actions/checkout",
        "actions/setup-node",
        "actions/download-artifact",
        "actions/upload-artifact",
      ]
      for (const s of stepsOf(wf).filter((x) => SIGN_JOBS.includes(x.job))) {
        if (s.uses && !allowed.includes(s.uses.split("@")[0])) p.push(`${label(s)}: uses ${s.uses}`)
        const run = s.run ?? ""
        for (const [, f] of run.matchAll(/\bnode (\S+)/g))
          if (!SIGN_NODE.includes(f)) p.push(`${label(s)}: node ${f}`)
        for (const [, f] of run.matchAll(/\bbash (\S+)/g))
          if (!SIGN_BASH.includes(f)) p.push(`${label(s)}: bash ${f}`)
        if (/\bnode\s+(?:-e|--eval|-p|--input-type)/.test(run)) p.push(`${label(s)}: inline node`)
        if (/\/(?:unsigned|out)\/[^"\s]*\/(?:MacOS|bin)\//.test(run))
          p.push(`${label(s)}: runs what it was handed`)
      }
      return p
    },

  "the signing job checks the build's sha256 before it unpacks anything": (wf) => {
    const p = []
    for (const [build, sign, file] of [
      ["build-linux", "sign-linux", "ranch-linux.tar.gz"],
      ["build-macos", "sign-macos", "ranch-macos.tar.gz"],
    ]) {
      const b = jobsOf(wf)[build]
      if (b?.outputs?.sha256 !== expr("steps.pack.outputs.sha256"))
        p.push(`${build}: no sha256 output`)
      const pack = b?.steps.find((s) => s.id === "pack")?.run ?? ""
      if (!pack.includes(`${file}" | cut -d' ' -f1)" >> "$GITHUB_OUTPUT"`))
        p.push(`${build}: the sha256 is not the packed file's`)
      const steps = jobsOf(wf)[sign]?.steps ?? []
      const i = steps.findIndex((s) => s.env?.SHA256 === `\${{ needs.${build}.outputs.sha256 }}`)
      const run = steps[i]?.run ?? ""
      const check = run.indexOf("shasum -a 256 -c")
      const unpack = run.search(/tar -xzf/)
      if (i < 0 || check < 0 || unpack < check || !/grep -Eqx '\[0-9a-f\]\{64\}'/.test(run))
        p.push(`${sign}: no sha256 check before the unpack`)
      const touch = steps.slice(0, i).filter((s) => (s.run ?? "").includes(file))
      if (touch.length) p.push(`${sign}: the artifact used before its check`)
    }
    return p
  },

  "the attach job checks each signing job's sums before it attaches": (wf) => {
    const s = jobsOf(wf).attach?.steps.find((x) => x.name === "The signing jobs' files")
    const want = {
      LINUX_SUMS: expr("needs.sign-linux.outputs.sums"),
      MACOS_SUMS: expr("needs.sign-macos.outputs.sums"),
    }
    const ok =
      JSON.stringify(s?.env) === JSON.stringify(want) &&
      /echo "\$LINUX_SUMS {2}SHA256SUMS-linux\.txt" \| shasum -a 256 -c\nshasum -a 256 -c SHA256SUMS-linux\.txt/.test(
        s.run,
      ) &&
      /echo "\$MACOS_SUMS {2}SHA256SUMS-macos\.txt" \| shasum -a 256 -c\nshasum -a 256 -c SHA256SUMS-macos\.txt/.test(
        s.run,
      )
    const steps = jobsOf(wf).attach?.steps ?? []
    const before = steps.indexOf(s) < steps.findIndex((x) => x.name === "Attach to the release")
    return ok && before ? [] : ["attach does not check the sums first"]
  },

  "the signing material is removed in an always() step, last": (wf) => {
    const p = []
    const mac = jobsOf(wf)["sign-macos"].steps.at(-1)
    if (
      mac.if !== "always()" ||
      !/bash scripts\/ci-keychain\.sh delete "\$RUNNER_TEMP\/signing"/.test(mac.run ?? "")
    )
      p.push("sign-macos")
    const linux = jobsOf(wf)["sign-linux"].steps.at(-1)
    if (linux.if !== "always()" || !/rm -rf "\$RUNNER_TEMP\/signing"/.test(linux.run ?? ""))
      p.push("sign-linux")
    return p
  },

  "the updater key: a 0600 file for the one signing command, removed after it": (wf) =>
    stepsOf(wf)
      .filter((s) => Object.values(secretsOf(s)).includes("TAURI_SIGNING_PRIVATE_KEY"))
      .flatMap((s) => {
        const r = s.run ?? ""
        const order = [
          "umask 077",
          `trap 'rm -f "$K"' EXIT`,
          'printf \'%s\\n\' "$TAURI_SIGNING_PRIVATE_KEY" > "$K"',
          "unset TAURI_SIGNING_PRIVATE_KEY",
          "node scripts/minisign.mjs sign",
        ]
        const at = order.map((x) => r.indexOf(x))
        const ok =
          at.every((x, i) => x >= 0 && (i === 0 || x > at[i - 1])) && /--key-file "\$K"/.test(r)
        return ok && (r.match(/\bnode /g) ?? []).length === 1
          ? []
          : [`${label(s)}: the key is not handled as a 0600 file for one command`]
      }),

  "the build jobs make no updater artifact, no DMG": (wf) => {
    const p = []
    for (const name of BUILD_JOBS) {
      const run = jobsOf(wf)
        [name].steps.map((s) => s.run ?? "")
        .join("\n")
      if (!run.includes(`CONFIG='{"bundle":{"createUpdaterArtifacts":false}}'`))
        p.push(`${name}: updater artifacts on`)
      for (const line of run.split("\n").filter((l) => /pnpm tauri (build|bundle)/.test(l))) {
        if (
          !/--config "\$CONFIG"|\\?"createUpdaterArtifacts\\?":false/.test(line) &&
          !/--config \\$/.test(line)
        )
          p.push(`${name}: ${line.trim()}`)
      }
      if (/--bundles [^\n]*dmg/.test(run)) p.push(`${name}: makes the DMG`)
    }
    return p
  },

  "the build jobs restore no cache: every package fetched afresh against the lockfile": (wf) =>
    BUILD_JOBS.flatMap((name) => {
      const steps = jobsOf(wf)[name].steps
      const pnpm = steps.filter((s) => s.uses === "./.github/actions/pnpm")
      const p = steps
        .filter((s) => s.uses?.startsWith("actions/cache@"))
        .map(() => `${name}: a cache`)
      if (pnpm.length !== 1 || pnpm[0].with?.cache !== "false")
        p.push(`${name}: the pnpm store cached`)
      return p
    }),

  "each job waits for the one before": (wf) => {
    const want = {
      release: undefined,
      "build-linux": "release",
      "build-macos": "release",
      "sign-linux": ["release", "build-linux"],
      "sign-macos": ["release", "build-macos"],
      attach: ["release", "sign-linux", "sign-macos"],
      publish: ["release", "attach"],
    }
    const got = Object.fromEntries(Object.entries(jobsOf(wf)).map(([k, j]) => [k, j.needs]))
    return JSON.stringify(got) === JSON.stringify(want) ? [] : [`needs: ${JSON.stringify(got)}`]
  },
}

const clone = () => structuredClone(WF)
/** For each rule, a workflow that breaks it (and the rule must say so). */
const MUTATIONS = {
  "a pushed v* tag is the only trigger": (wf) => {
    wf.on.pull_request = null
  },
  "reads by default; contents: write only on release, attach, publish": (wf) => {
    wf.jobs["sign-macos"].permissions.contents = "write"
  },
  "a build job holds no secret, no variable, no token, no environment": (wf) => {
    wf.jobs["build-macos"].steps.find((s) => s.name === "Build the app").env = {
      APPLE_SIGNING_IDENTITY: expr("secrets.APPLE_SIGNING_IDENTITY"),
    }
  },
  "environment: release on release, the signing jobs and publish, nowhere else": (wf) => {
    delete wf.jobs["sign-linux"].environment
  },
  "secrets only in a step's env, by value or presence; the release job sees presence only": (
    wf,
  ) => {
    wf.jobs["sign-macos"].env = { K: expr("secrets.TAURI_SIGNING_PRIVATE_KEY") }
  },
  "each secret's value only in the step that uses it": (wf) => {
    step(wf, "sign-macos", "Collect the files").env.APPLE_CERTIFICATE = expr(
      "secrets.APPLE_CERTIFICATE",
    )
  },
  "every action pinned to an exact release's commit, the tag beside it; the local action in build jobs only":
    (wf) => {
      wf.jobs["sign-linux"].steps.splice(2, 0, { uses: "./.github/actions/pnpm" })
    },
  "every job checks out with persist-credentials: false": (wf) => {
    wf.jobs.attach.steps[0].with["persist-credentials"] = true
  },
  "no ${{ }} inside a script: values come through env": (wf) => {
    step(wf, "attach", "Attach to the release").run += `echo "${expr("github.ref_name")}"\n`
  },
  "the job token only in steps that call gh, never beside a signing secret": (wf) => {
    step(wf, "sign-macos", "Collect the files").env.GH_TOKEN = expr("github.token")
  },
  "no build tool outside the build jobs": (wf) => {
    step(wf, "attach", "Attach to the release").run += "pnpm install --frozen-lockfile\n"
  },
  "a signing job uses GitHub's checkout, Node and artifact actions, and runs this repository's signing scripts only":
    (wf) => {
      step(wf, "sign-linux", "Collect the files").run += 'node "$RUNNER_TEMP/out/postinstall.js"\n'
    },
  "the signing job checks the build's sha256 before it unpacks anything": (wf) => {
    const s = step(wf, "sign-macos", "The build job's app")
    s.run = s.run.replace(/echo "\$SHA256 {2}\$A" \| shasum -a 256 -c\n/, "")
  },
  "the attach job checks each signing job's sums before it attaches": (wf) => {
    const s = step(wf, "attach", "The signing jobs' files")
    s.run = s.run.replace("shasum -a 256 -c SHA256SUMS-macos.txt", "true")
  },
  "the signing material is removed in an always() step, last": (wf) => {
    delete wf.jobs["sign-macos"].steps.at(-1).if
  },
  "the updater key: a 0600 file for the one signing command, removed after it": (wf) => {
    const s = step(wf, "sign-linux", "Sign the AppImage")
    s.run = s.run.replace(`trap 'rm -f "$K"' EXIT\n`, "")
  },
  "the build jobs make no updater artifact, no DMG": (wf) => {
    const s = step(wf, "build-macos", "Build the app")
    s.run = s.run.replace("--bundles app --config", "--bundles app,dmg --config")
  },
  "the build jobs restore no cache: every package fetched afresh against the lockfile": (wf) => {
    delete wf.jobs["build-linux"].steps.find((s) => s.uses === "./.github/actions/pnpm").with
  },
  "each job waits for the one before": (wf) => {
    wf.jobs["sign-macos"].needs = ["release"]
  },
}

describe("release.yml: its rules, each caught by its mutation", () => {
  for (const [name, rule] of Object.entries(RULES)) {
    test(name, () => {
      assert.deepEqual(rule(WF), [])
      const mutate = MUTATIONS[name]
      assert.ok(mutate, `no mutation for "${name}"`)
      const wf = clone()
      mutate(wf)
      assert.notDeepEqual(rule(wf), [], `the mutation of "${name}" went unnoticed`)
    })
  }

  test("every rule has its mutation, and only rules have one", () => {
    assert.deepEqual(Object.keys(MUTATIONS).sort(), Object.keys(RULES).sort())
  })

  test("the pins: a moved tag, a short tag or a branch in a comment is caught", () => {
    const pin =
      RULES[
        "every action pinned to an exact release's commit, the tag beside it; the local action in build jobs only"
      ]
    const mutated = (from, to) => pin(WF, [raw.replace(from, to), composite])
    assert.notDeepEqual(
      mutated(
        "018cc2cf5baa6db3ef3c5f8a56943fffe632ef53 # v6.0.0",
        "018cc2cf5baa6db3ef3c5f8a56943fffe632ef53 # v6",
      ),
      [],
    )
    assert.notDeepEqual(mutated("# v5.1.0", "# v5.2.0"), [])
    assert.notDeepEqual(pin(WF, [raw, composite.replace("# v4.3.0", "# v4")]), [])
    assert.notDeepEqual(
      mutated(
        "actions/checkout@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09 # v5.1.0",
        "dtolnay/rust-toolchain@stable",
      ),
      [],
    )
  })
})

/** The files a module loads, and its imports of anything but Node.js built-ins and repository files. */
function importGraph(entry) {
  const files = new Set()
  const bad = []
  const visit = (file) => {
    if (files.has(file)) return
    files.add(file)
    const src = readFileSync(file, "utf8")
    const specs = [
      ...[...src.matchAll(/^\s*(?:import|export)\s[^;]*?\sfrom\s+["']([^"']+)["']/gm)].map(
        (m) => m[1],
      ),
      ...[...src.matchAll(/^\s*import\s+["']([^"']+)["']/gm)].map((m) => m[1]),
      ...[...src.matchAll(/\b(?:import|require)\(\s*["']([^"']+)["']\s*\)/g)].map((m) => m[1]),
    ]
    for (const m of src.matchAll(/\b(?:import|require)\(\s*(?!["'])([^)]*)\)/g))
      bad.push(`${relative(REPO, file)}: ${m[0]}`)
    if (/\bcreateRequire\b/.test(src)) bad.push(`${relative(REPO, file)}: createRequire`)
    for (const spec of specs) {
      if (spec.startsWith("node:")) continue
      if (spec.startsWith("./") || spec.startsWith("../")) visit(resolve(dirname(file), spec))
      else bad.push(`${relative(REPO, file)}: ${spec}`)
    }
  }
  visit(entry)
  return { files: [...files].map((f) => relative(REPO, f)).sort(), bad }
}

describe("what the jobs after the builds run: Node.js built-ins and this repository, no package", () => {
  test("every `node <script>` outside the build jobs, and the node scripts of attach-release.sh, import no package", () => {
    const entries = new Set()
    for (const s of stepsOf(WF).filter((x) => !BUILD_JOBS.includes(x.job))) {
      for (const [, f] of scriptOf(s).matchAll(
        /\bnode "?(?:\$here\/)?((?:scripts\/)?[\w.-]+\.mjs)/g,
      )) {
        entries.add(f.startsWith("scripts/") ? f : `scripts/${f}`)
      }
      for (const [, f] of (s.run ?? "").matchAll(/from "\.\/(scripts\/[\w.-]+\.mjs)"/g))
        entries.add(f)
    }
    assert.deepEqual([...entries].sort(), [
      "scripts/homebrew-cask.mjs",
      "scripts/minisign.mjs",
      "scripts/publish-update.mjs",
      "scripts/release-preflight.mjs",
      "scripts/set-version.mjs",
      "scripts/sign-macos.mjs",
    ])
    for (const e of entries) {
      const g = importGraph(join(REPO, e))
      assert.deepEqual(g.bad, [], e)
      for (const f of g.files) assert.ok(!f.startsWith("..") && !f.includes("node_modules"), f)
    }
  })

  test("the walk finds a package import, a nested one and a dynamic one (canaries)", () => {
    const d = fresh("graph")
    writeFileSync(join(d, "a.mjs"), 'import { x } from "./b.mjs"\nimport fs from "node:fs"\n')
    writeFileSync(
      join(d, "b.mjs"),
      'export const x = 1\nimport "yaml"\nconst m = await import(name)\n',
    )
    const g = importGraph(join(d, "a.mjs"))
    assert.equal(g.files.length, 2)
    assert.deepEqual(
      g.bad.map((b) => b.split(": ")[1]),
      ["import(name)", "yaml"],
    )
  })
})

/**
 * A step's script run as GitHub runs it (bash --noprofile --norc -eo
 * pipefail), in the checkout, with RUNNER_TEMP and GITHUB_OUTPUT of a scratch
 * folder and the stand-ins first on PATH.
 */
function runStep(s, env = {}, bin) {
  const temp = env.RUNNER_TEMP ?? fresh("runner-temp")
  const output = join(temp, "github-output")
  writeFileSync(output, "", { flag: "a" })
  const r = spawnSync("/bin/bash", ["--noprofile", "--norc", "-eo", "pipefail", "-c", s.run], {
    cwd: REPO,
    encoding: "utf8",
    env: {
      PATH: `${bin ? `${bin}:` : ""}${dirname(process.execPath)}:/usr/bin:/bin`,
      HOME: tmp,
      RUNNER_TEMP: temp,
      GITHUB_OUTPUT: output,
      GITHUB_REF_NAME: "v0.5.0",
      GITHUB_ACTIONS: "true",
      ...env,
    },
  })
  const outputs = Object.fromEntries(
    readFileSync(output, "utf8")
      .split("\n")
      .filter((l) => l.includes("="))
      .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
  )
  return { ...r, temp, outputs }
}

describe("release.yml's steps, run against stand-ins", () => {
  test("What is configured: nothing → each set off and said; everything → on; a partial set fails naming what is missing", () => {
    const s = step(WF, "release", "What is configured")
    const names = Object.values(SETS).flatMap((x) => x.names)
    assert.deepEqual(
      Object.keys(s.env)
        .filter((k) => !["HAS_APPLE_ID", "HAS_APPLE_PASSWORD"].includes(k))
        .sort(),
      names.map((x) => `HAS_${x}`).sort(),
    )
    for (const [k, v] of Object.entries(s.env))
      assert.equal(v, `\${{ secrets.${k.slice(4)} != '' }}`)
    const none = Object.fromEntries(Object.keys(s.env).map((k) => [k, "false"]))
    const off = runStep(s, none)
    assert.equal(off.status, 0, off.stderr)
    assert.deepEqual(off.outputs, { signed: "false", updates: "off", tap: "off" })
    assert.match(off.stdout, /::warning::No APPLE_CERTIFICATE, .*: the macOS app is signed ad hoc/)
    const all = Object.fromEntries(names.map((x) => [`HAS_${x}`, "true"]))
    const on = runStep(s, { ...none, ...all, HAS_APPLE_PASSWORD: "true" })
    assert.deepEqual(on.outputs, { signed: "true", updates: "on", tap: "on" })
    assert.match(on.stdout, /::warning::APPLE_PASSWORD is no longer used/)
    const partial = runStep(s, {
      ...none,
      ...all,
      HAS_APPLE_API_KEY_P8: "false",
      HAS_TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "false",
    })
    assert.equal(partial.status, 1)
    assert.match(
      partial.stdout,
      /::error::the Developer ID signature .* or none: missing APPLE_API_KEY_P8/,
    )
    assert.match(
      partial.stdout,
      /::error::the updater archives' signatures .* or none: missing TAURI_SIGNING_PRIVATE_KEY_PASSWORD/,
    )
    assert.deepEqual(partial.outputs, {})
  })

  for (const [job, name, file] of [
    ["sign-macos", "The build job's app", "ranch-macos.tar.gz"],
    ["sign-linux", "The build job's bundles", "ranch-linux.tar.gz"],
  ]) {
    test(`${job} › ${name}: the packed file with its sha256 is unpacked; another sha256, or none, unpacks nothing`, () => {
      const s = step(WF, job, name)
      const temp = fresh("check")
      const src = fresh("packed")
      writeFileSync(join(src, "Ranch_0.5.0_amd64.deb"), "a deb")
      mkdirSync(join(temp, "artifact"))
      const tar = join(temp, "artifact", file)
      assert.equal(spawnSync("tar", ["-czf", tar, "-C", src, "."]).status, 0)
      const into = join(temp, job === "sign-macos" ? "unsigned" : "out")
      for (const bad of ["0".repeat(64), "", "not-a-sum"]) {
        const r = runStep(s, { RUNNER_TEMP: temp, SHA256: bad })
        assert.notEqual(r.status, 0, bad)
        assert.ok(!existsSync(join(into, "Ranch_0.5.0_amd64.deb")), bad)
      }
      const ok = runStep(s, { RUNNER_TEMP: temp, SHA256: sha256(readFileSync(tar)) })
      assert.equal(ok.status, 0, ok.stderr)
      assert.equal(readFileSync(join(into, "Ranch_0.5.0_amd64.deb"), "utf8"), "a deb")
    })
  }

  for (const [job, name, target] of [
    ["sign-linux", "Sign the AppImage", "out/Ranch_0.5.0_amd64.AppImage"],
    ["sign-macos", "Sign the update archive", "out/Ranch.app.tar.gz"],
  ]) {
    test(`${job} › ${name}: the key reaches the signer as a 0600 file, gone after it, even when it fails`, () => {
      const s = step(WF, job, name)
      assert.equal(s.if, "needs.release.outputs.updates == 'on'")
      for (const exit of [0, 1]) {
        const temp = fresh("updater-key")
        mkdirSync(join(temp, "out"))
        writeFileSync(join(temp, target), "archive")
        const fakes = fakeBin(fresh("fake-node"), {
          node: `
const k = argv[argv.indexOf("--key-file") + 1]
fs.writeFileSync(state + "/seen.json", JSON.stringify({ mode: fs.statSync(k).mode & 0o777, key: fs.readFileSync(k, "utf8"), envKey: env.TAURI_SIGNING_PRIVATE_KEY ?? null, password: env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }))
process.exit(${exit})`,
        })
        const r = runStep(
          s,
          {
            RUNNER_TEMP: temp,
            VERSION: "0.5.0",
            TAURI_SIGNING_PRIVATE_KEY: "updater-key-canary",
            TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "pw",
          },
          fakes.bin,
        )
        assert.equal(r.status, exit, r.stderr)
        const [call] = fakes.calls()
        const k = join(temp, "signing", "updater.key")
        assert.deepEqual(call.argv, [
          "scripts/minisign.mjs",
          "sign",
          join(temp, target),
          "--key-file",
          k,
          "--version",
          "0.5.0",
        ])
        assert.deepEqual(JSON.parse(readFileSync(join(fakes.state, "seen.json"), "utf8")), {
          mode: 0o600,
          key: "updater-key-canary\n",
          envKey: null,
          password: "pw",
        })
        assert.ok(!existsSync(k), "the key file stays")
        assert.doesNotMatch(r.stdout + r.stderr, /canary/)
      }
    })
  }

  test("sign-macos › Signing material: without the Apple set nothing is imported; with it the keychain and a 0600 .p8", () => {
    const s = step(WF, "sign-macos", "Signing material")
    const fakes = fakeBin(fresh("fake-bash"), {
      bash: `fs.writeFileSync(state + "/argv.json", JSON.stringify(argv)); console.log("keychain=/tmp/kc")`,
    })
    const off = runStep(s, { SIGNED: "false", APPLE_API_KEY_P8: "" }, fakes.bin)
    assert.equal(off.status, 0, off.stderr)
    assert.deepEqual(fakes.calls(), [])
    assert.deepEqual(readdirSync(join(off.temp, "signing")), [])
    const on = runStep(s, { SIGNED: "true", APPLE_API_KEY_P8: "p8-canary" }, fakes.bin)
    assert.equal(on.status, 0, on.stderr)
    assert.deepEqual(fakes.calls()[0].argv, [
      "scripts/ci-keychain.sh",
      "import",
      join(on.temp, "signing"),
    ])
    assert.equal(on.outputs.keychain, "/tmp/kc")
    assert.equal(on.outputs.api_key_path, join(on.temp, "signing", "AuthKey.p8"))
    assert.equal(statSync(on.outputs.api_key_path).mode & 0o777, 0o600)
    assert.equal(statSync(join(on.temp, "signing")).mode & 0o777, 0o700)
  })

  test("sign-macos › Collect the files: the DMG always, the archive and its .sig with updates on; one missing fails", () => {
    const s = step(WF, "sign-macos", "Collect the files")
    const make = (names) => {
      const temp = fresh("collect")
      mkdirSync(join(temp, "out"))
      for (const f of [...names, "dmg-root"]) writeFileSync(join(temp, "out", f), f)
      return temp
    }
    const all = ["Ranch_0.5.0_universal.dmg", "Ranch.app.tar.gz", "Ranch.app.tar.gz.sig"]
    const on = runStep(s, { UPDATES: "on", VERSION: "0.5.0", RUNNER_TEMP: make(all) })
    assert.equal(on.status, 0, on.stderr)
    assert.deepEqual(
      readdirSync(join(on.temp, "release")).sort(),
      [...all, "SHA256SUMS-macos.txt"].sort(),
    )
    assert.equal(
      on.outputs.sums,
      sha256(readFileSync(join(on.temp, "release", "SHA256SUMS-macos.txt"))),
    )
    const off = runStep(s, { UPDATES: "off", VERSION: "0.5.0", RUNNER_TEMP: make(all) })
    assert.deepEqual(readdirSync(join(off.temp, "release")).sort(), [
      "Ranch_0.5.0_universal.dmg",
      "SHA256SUMS-macos.txt",
    ])
    const unsigned = runStep(s, {
      UPDATES: "on",
      VERSION: "0.5.0",
      RUNNER_TEMP: make(all.slice(0, 2)),
    })
    assert.equal(unsigned.status, 1)
    assert.match(unsigned.stdout, /::error::Ranch\.app\.tar\.gz\.sig was not made \(updates: on\)/)
  })

  test("sign-linux › Collect the files: one .deb, .rpm and AppImage, the .sig with updates on; missing or doubled fails", () => {
    const s = step(WF, "sign-linux", "Collect the files")
    const make = (names) => {
      const temp = fresh("collect-linux")
      mkdirSync(join(temp, "out"))
      for (const f of names) writeFileSync(join(temp, "out", f), f)
      return temp
    }
    const base = ["Ranch_0.5.0_amd64.deb", "Ranch-0.5.0-1.x86_64.rpm", "Ranch_0.5.0_amd64.AppImage"]
    const on = runStep(s, {
      UPDATES: "on",
      RUNNER_TEMP: make([...base, "Ranch_0.5.0_amd64.AppImage.sig"]),
    })
    assert.equal(on.status, 0, on.stderr)
    assert.equal(readdirSync(join(on.temp, "release")).length, 5)
    const off = runStep(s, { UPDATES: "off", RUNNER_TEMP: make(base) })
    assert.equal(off.status, 0, off.stderr)
    assert.ok(!readdirSync(join(off.temp, "release")).some((f) => f.endsWith(".sig")))
    assert.equal(runStep(s, { UPDATES: "on", RUNNER_TEMP: make(base) }).status, 1)
    assert.equal(runStep(s, { UPDATES: "off", RUNNER_TEMP: make(base.slice(1)) }).status, 1)
    assert.equal(
      runStep(s, { UPDATES: "off", RUNNER_TEMP: make([...base, "Ranch_0.5.1_amd64.deb"]) }).status,
      1,
    )
  })

  test("attach › The signing jobs' files: a sums file other than the signing job's, or a file other than its sums, fails", () => {
    const s = step(WF, "attach", "The signing jobs' files")
    const temp = fresh("attach-check")
    const sums = {}
    for (const p of ["linux", "macos"]) {
      const d = join(temp, "release", p)
      mkdirSync(d, { recursive: true })
      writeFileSync(join(d, "file"), p)
      writeFileSync(join(d, `SHA256SUMS-${p}.txt`), `${sha256(p)}  file\n`)
      sums[p] = sha256(readFileSync(join(d, `SHA256SUMS-${p}.txt`)))
    }
    const ok = runStep(s, { RUNNER_TEMP: temp, LINUX_SUMS: sums.linux, MACOS_SUMS: sums.macos })
    assert.equal(ok.status, 0, ok.stderr)
    assert.notEqual(
      runStep(s, { RUNNER_TEMP: temp, LINUX_SUMS: sums.linux, MACOS_SUMS: sums.linux }).status,
      0,
    )
    writeFileSync(join(temp, "release", "macos", "file"), "swapped")
    assert.notEqual(
      runStep(s, { RUNNER_TEMP: temp, LINUX_SUMS: sums.linux, MACOS_SUMS: sums.macos }).status,
      0,
    )
  })

  test("publish › Publish the updates: with updates on, a missing signature fails instead of skipping the update", () => {
    const s = step(WF, "publish", "Publish the updates")
    assert.equal(s.if, "needs.release.outputs.updates == 'on'")
    const rel = fresh("release-assets")
    writeFileSync(join(rel, "Ranch_0.5.0_amd64.AppImage.sig"), "sig")
    const fakes = fakeBin(fresh("fake-gh"), {
      gh: `
const opt = (f) => argv.filter((x, i) => argv[i - 1] === f)
if (argv[1] === "download") for (const p of opt("--pattern")) for (const f of fs.readdirSync(env.FAKE_RELEASE_DIR)) {
  if (new RegExp("^" + p.replace(/\\./g, "\\\\.").replace(/\\*/g, ".*") + "$").test(f)) fs.copyFileSync(path.join(env.FAKE_RELEASE_DIR, f), path.join(opt("--dir")[0], f))
}`,
      node: "",
    })
    const temp = fresh("publish")
    const run = (cwd) =>
      spawnSync("/bin/bash", ["--noprofile", "--norc", "-eo", "pipefail", "-c", s.run], {
        cwd,
        encoding: "utf8",
        env: {
          PATH: `${fakes.bin}:/usr/bin:/bin`,
          GITHUB_REF_NAME: "v0.5.0",
          FAKE_RELEASE_DIR: rel,
        },
      })
    const missing = run(temp)
    assert.equal(missing.status, 1)
    assert.match(missing.stdout, /::error::updates are on, but the release lacks/)
    assert.deepEqual(
      fakes.calls().filter((c) => c.tool === "node"),
      [],
    )
    writeFileSync(join(rel, "Ranch.app.tar.gz.sig"), "sig")
    const ok = run(fresh("publish"))
    assert.equal(ok.status, 0, ok.stderr)
    assert.deepEqual(
      fakes
        .calls()
        .filter((c) => c.tool === "node")
        .map((c) => c.argv.slice(0, 4)),
      [
        ["scripts/publish-update.mjs", "v0.5.0", "--platforms", "linux-x86_64"],
        ["scripts/publish-update.mjs", "v0.5.0", "--platforms", "darwin-aarch64,darwin-x86_64"],
      ],
    )
  })

  test("every script the workflow names exists", () => {
    const paths = new Set([...raw.matchAll(/scripts\/[\w.-]+\.(?:mjs|sh)/g)].map((m) => m[0]))
    assert.ok(paths.size >= 8)
    for (const p of paths) assert.ok(existsSync(join(REPO, p)), p)
  })
})
