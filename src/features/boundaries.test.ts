import { describe, expect, it } from "vitest"

/**
 * The shape of `src/`, checked on every run:
 *
 * - A feature is used through its `index.ts` only (`@/features/jobs`, never
 *   `@/features/jobs/JobList`), so its internals can move freely.
 * - The shared layers (`data`, `domain`, `lib`, `components`, `stores`,
 *   `hooks`, `commands`, `backend`, `capabilities`) never import a feature.
 * - Features do not depend on each other in a circle: what two features
 *   share belongs in a shared layer.
 * - A backend command that `src/data` wraps is called from there only, so
 *   its query keys, invalidation and error handling live in one place.
 * - Features and routes never import the bindings' `commands`: every call
 *   they make goes through `src/data`.
 */

const sources = import.meta.glob<string>(["/src/**/*.{ts,tsx}", "!/src/**/*.d.ts"], {
  query: "?raw",
  import: "default",
  eager: true,
})

const SHARED_LAYERS = [
  "data",
  "domain",
  "lib",
  "components",
  "stores",
  "hooks",
  "commands",
  "backend",
  "capabilities",
]

interface Import {
  file: string
  spec: string
  /** The imported module as a path under `src/`, without extension. */
  target: string
}

function resolve(file: string, spec: string): string | null {
  if (spec.startsWith("@/")) return `src/${spec.slice(2)}`
  if (!spec.startsWith(".")) return null
  const parts = file.split("/").slice(0, -1)
  for (const part of spec.split("/")) {
    if (part === "..") parts.pop()
    else if (part !== ".") parts.push(part)
  }
  return parts.join("/")
}

function importsOf(file: string, source: string): Import[] {
  const found: Import[] = []
  const pattern = /(?:from\s+|import\s*\(\s*|^import\s+)"([^"]+)"/gm
  for (const match of source.matchAll(pattern)) {
    const spec = match[1]
    const target = resolve(file, spec)
    if (target) found.push({ file, spec, target })
  }
  return found
}

/** `src/features/jobs/JobList` → `["jobs", "JobList"]`; anything else → null. */
function featurePath(path: string): [string, string] | null {
  const match = /^src\/features\/([^/]+)(?:\/(.+))?$/.exec(path)
  if (!match || match[1].includes(".")) return null
  return [match[1], match[2] ?? ""]
}

const imports = Object.entries(sources).flatMap(([absolute, source]) =>
  importsOf(absolute.replace(/^\//, ""), source),
)

describe("feature boundaries", () => {
  it("reach another feature through its index only", () => {
    const deep = imports.filter(({ file, target }) => {
      const to = featurePath(target)
      if (!to) return false
      const from = featurePath(file)
      if (from && from[0] === to[0]) return false
      return to[1] !== "" && to[1] !== "index"
    })
    expect(deep.map(({ file, spec }) => `${file} → ${spec}`)).toEqual([])
  })

  it("keep the shared layers free of features", () => {
    const upward = imports.filter(
      ({ file, target }) =>
        SHARED_LAYERS.some((layer) => file.startsWith(`src/${layer}/`)) &&
        featurePath(target) !== null,
    )
    expect(upward.map(({ file, spec }) => `${file} → ${spec}`)).toEqual([])
  })

  it("form no circle between features", () => {
    const edges = new Map<string, Set<string>>()
    for (const { file, target } of imports) {
      const from = featurePath(file)
      const to = featurePath(target)
      if (!from || !to || from[0] === to[0]) continue
      if (!edges.has(from[0])) edges.set(from[0], new Set())
      edges.get(from[0])?.add(to[0])
    }
    const cycles: string[] = []
    const done = new Set<string>()
    const visit = (node: string, path: string[]) => {
      if (path.includes(node)) {
        cycles.push([...path.slice(path.indexOf(node)), node].join(" → "))
        return
      }
      if (done.has(node)) return
      for (const next of edges.get(node) ?? []) visit(next, [...path, node])
      done.add(node)
    }
    for (const node of edges.keys()) visit(node, [])
    expect(cycles).toEqual([])
  })

  it("call a backend command only from its data module, once it has one", () => {
    const callsOf = (source: string) =>
      new Set([...source.matchAll(/\bcommands\.(\w+)\(/g)].map((m) => m[1]))
    const isProduction = (file: string) => !/\.test\.tsx?$/.test(file)
    const owned = new Set<string>()
    for (const [file, source] of Object.entries(sources)) {
      if (file.startsWith("/src/data/") && isProduction(file)) {
        for (const command of callsOf(source)) owned.add(command)
      }
    }
    const strays = Object.entries(sources).flatMap(([file, source]) =>
      file.startsWith("/src/data/") || !isProduction(file)
        ? []
        : [...callsOf(source)].filter((c) => owned.has(c)).map((c) => `${file} → commands.${c}`),
    )
    expect(strays).toEqual([])
  })

  it("leave the backend commands to src/data in features and routes", () => {
    // A screen reads and writes through `src/data`; a file that truly must
    // call a command itself goes here, with the reason.
    const allowed: Record<string, string> = {}
    const isScreen = (file: string) =>
      /^src\/(features|routes)\//.test(file) && !/\.test\.tsx?$/.test(file)
    const pattern = /import\s+(type\s+)?(\*\s+as\s+\w+|\{[^}]*\})\s+from\s+"([^"]+)"/g
    const strays = Object.entries(sources).flatMap(([absolute, source]) => {
      const file = absolute.replace(/^\//, "")
      if (!isScreen(file) || file in allowed) return []
      return [...source.matchAll(pattern)]
        .filter(([, typeOnly, names, spec]) => {
          if (typeOnly || resolve(file, spec) !== "src/lib/tauri/bindings") return false
          if (names.startsWith("*")) return true
          return names
            .slice(1, -1)
            .split(",")
            .some((name) => /^commands(\s+as\s+\w+)?$/.test(name.trim()))
        })
        .map(() => file)
    })
    expect(strays).toEqual([])
    expect(Object.keys(allowed).filter((file) => !(`/${file}` in sources))).toEqual([])
  })

  it("sees the source tree it checks", () => {
    expect(imports.length).toBeGreaterThan(1000)
    expect(imports.some(({ target }) => featurePath(target) !== null)).toBe(true)
  })
})
