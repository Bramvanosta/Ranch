// No text is cut, ellipsised or split mid-word on purpose (docs/design.md,
// "Rules every screen follows"). The overflow check only sees what the demo
// world happens to overflow; this one refuses the classes that would cut a
// longer name than the demo's, wherever they are.

import { readdirSync, readFileSync } from "node:fs"
import { join } from "node:path"
import { describe, expect, it } from "vitest"

const FORBIDDEN: Array<[RegExp, string]> = [
  [/\btruncate\b/, "`truncate` ends the text with an ellipsis: let it wrap"],
  [/\bline-clamp-\d/, "`line-clamp` cuts the text after N lines: let it wrap"],
  [/\btext-ellipsis\b|text-overflow:\s*ellipsis/, "an ellipsis cuts the text: let it wrap"],
  [
    /\bbreak-all\b|word-break:\s*break-all/,
    "`break-all` splits words anywhere: use `wrap-anywhere`",
  ],
]

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) return sources(path)
    return /\.(tsx?|css)$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) ? [path] : []
  })
}

describe("no clipped text", () => {
  it("no source file asks for text to be cut", () => {
    const found: string[] = []
    for (const file of sources("src")) {
      readFileSync(file, "utf8")
        .split("\n")
        .forEach((line, i) => {
          // Comments may name the classes they avoid.
          if (/^\s*(\/\/|\*|\/\*)/.test(line)) return
          for (const [pattern, why] of FORBIDDEN) {
            if (pattern.test(line)) found.push(`${file}:${i + 1}: ${why}`)
          }
        })
    }
    expect(found).toEqual([])
  })
})
