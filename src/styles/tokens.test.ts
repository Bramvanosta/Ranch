// Contrast of the design tokens: read globals.css as
// text, resolve each theme's custom properties, and check WCAG ratios.

import { readdirSync, readFileSync } from "node:fs"
import { describe, expect, it } from "vitest"

// Vitest stubs CSS imports (even `?raw`), so read the file; tests run from the repo root.
const css = readFileSync("src/styles/globals.css", "utf8")

/** The custom properties declared directly in the first block opened by `selector`. */
function block(selector: string): Record<string, string> {
  const start = css.indexOf(`\n${selector} {`)
  if (start === -1) throw new Error(`no ${selector} block in globals.css`)
  const body = css.slice(css.indexOf("{", start) + 1, css.indexOf("\n}", start))
  const vars: Record<string, string> = {}
  for (const m of body.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    vars[m[1] as string] = (m[2] as string).trim()
  }
  return vars
}

const light = block(":root")
const themes = { light, dark: { ...light, ...block(".dark") } }
type Theme = keyof typeof themes

type Rgb = [number, number, number]

function resolve(theme: Theme, value: string, depth = 0): string {
  const ref = value.match(/^var\((--[\w-]+)\)$/)
  if (!ref) return value
  const next = themes[theme][ref[1] as string]
  if (next === undefined || depth > 8) throw new Error(`${theme}: ${ref[1]} is not defined`)
  return resolve(theme, next, depth + 1)
}

function oklchToRgb(l: number, c: number, hDeg: number): Rgb {
  const h = (hDeg * Math.PI) / 180
  const a = c * Math.cos(h)
  const b = c * Math.sin(h)
  const l_ = (l + 0.3963377774 * a + 0.2158037573 * b) ** 3
  const m_ = (l - 0.1055613458 * a - 0.0638541728 * b) ** 3
  const s_ = (l - 0.0894841775 * a - 1.291485548 * b) ** 3
  const lin: Rgb = [
    4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
    -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
    -0.0041960863 * l_ - 0.7034186147 * m_ + 1.707614701 * s_,
  ]
  // Back to gamma-encoded sRGB, clipped to the gamut the screen can show.
  return lin.map((x) => {
    const v = Math.min(1, Math.max(0, x))
    return v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055
  }) as Rgb
}

/** An opaque colour as gamma-encoded sRGB in 0..1. */
function color(theme: Theme, name: string): Rgb {
  const value = resolve(theme, themes[theme][name] ?? `var(${name})`)
  const hex = value.match(/^#([0-9a-f]{6})$/i)
  if (hex) {
    const n = Number.parseInt(hex[1] as string, 16)
    return [(n >> 16) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255]
  }
  const ok = value.match(/^oklch\(\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)\s*\)$/)
  if (ok) return oklchToRgb(Number(ok[1]), Number(ok[2]), Number(ok[3]))
  throw new Error(`${theme}: ${name} = "${value}" is not an opaque hex or oklch colour`)
}

const luminance = (rgb: Rgb) => {
  const [r, g, b] = rgb.map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
  return 0.2126 * (r as number) + 0.7152 * (g as number) + 0.0722 * (b as number)
}

const contrast = (theme: Theme, fg: string, bg: string) => {
  const [hi, lo] = [luminance(color(theme, fg)), luminance(color(theme, bg))].sort((x, y) => y - x)
  return ((hi as number) + 0.05) / ((lo as number) + 0.05)
}

const STATES = ["waiting", "active", "delayed", "failed", "completed"]
const ENVS = [
  "production",
  "staging",
  "local",
  ...Array.from({ length: 7 }, (_, i) => `custom-${i + 1}`),
]

describe.each(["light", "dark"] as Theme[])("%s theme", (theme) => {
  const pairs: [string, string][] = [
    ...["--text", "--text2"].flatMap((fg) =>
      ["--bg", "--surf", "--side"].map((bg): [string, string] => [fg, bg]),
    ),
    ...STATES.map((s): [string, string] => [`--state-${s}-text`, "--bg"]),
    ["--accent-text", "--accent"],
    ["--ai-text", "--ai"],
    ["--danger-text", "--danger"],
    ["--link", "--bg"],
    // Payloads and code sit on --field (inputs, code blocks) or --surf (panels).
    ...["--syntax-key", "--syntax-string", "--syntax-literal"].flatMap((fg) =>
      ["--field", "--surf", "--bg"].map((bg): [string, string] => [fg, bg]),
    ),
  ]

  it.each(pairs)("%s on %s reaches 4.5:1", (fg, bg) => {
    expect(contrast(theme, fg, bg)).toBeGreaterThanOrEqual(4.5)
  })

  // Environment dots are graphics, not text: 3:1 (WCAG 1.4.11).
  it.each(
    ENVS.flatMap((e) => ["--bg", "--side"].map((bg) => [`--env-${e}`, bg])),
  )("%s on %s reaches 3:1", (env, bg) => {
    expect(contrast(theme, env as string, bg as string)).toBeGreaterThanOrEqual(3)
  })
})

// The migration aliases are gone: no shadcn colour
// variable is declared, and no utility in the code names one.
it("declares no shadcn alias any more", () => {
  for (const alias of [
    "background",
    "foreground",
    "card",
    "popover",
    "primary",
    "muted",
    "destructive",
    "border",
    "input",
    "ring",
    "sidebar",
  ]) {
    expect(css, alias).not.toMatch(new RegExp(`--${alias}:`))
    expect(css, alias).not.toMatch(new RegExp(`--color-${alias}:`))
  }
  expect(css).not.toMatch(/--color-accent:/)
})

it("no utility uses a shadcn colour name", () => {
  const shadcn =
    /(?<![\w\-(])(?:bg|text|border|ring|outline|fill|stroke|divide)-(?:background|foreground|card|popover|primary|secondary|muted|muted-foreground|accent|accent-foreground|destructive|input|ring|sidebar)(?![\w-])/
  const offenders = readdirSync("src", { recursive: true, encoding: "utf8" })
    .filter((f) => /\.(tsx?|css)$/.test(f) && !f.endsWith(".test.ts") && !f.endsWith(".test.tsx"))
    .filter((f) => shadcn.test(readFileSync(`src/${f}`, "utf8")))
  expect(offenders).toEqual([])
})

describe("type", () => {
  const sources = readdirSync("src", { recursive: true, encoding: "utf8" })
    .filter((f) => /\.(tsx?|css)$/.test(f) && !f.endsWith(".test.ts") && !f.endsWith(".test.tsx"))
    .map((f) => ({ file: f, text: readFileSync(`src/${f}`, "utf8") }))

  it("no text is set under 11 px", () => {
    const tooSmall = sources.flatMap(({ file, text }) =>
      [...text.matchAll(/text-\[(\d+(?:\.\d+)?)px\]/g)]
        .filter((m) => Number(m[1]) < 11)
        .map((m) => `${file}: ${m[0]}`),
    )
    expect(tooSmall).toEqual([])
  })

  it("no font is fetched from a CDN", () => {
    const index = readFileSync("index.html", "utf8")
    for (const { file, text } of [...sources, { file: "index.html", text: index }]) {
      expect(text, file).not.toMatch(/fonts\.(googleapis|gstatic)\.com|use\.typekit|fonts\.bunny/)
    }
  })
})
