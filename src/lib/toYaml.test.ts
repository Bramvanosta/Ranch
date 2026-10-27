import { describe, expect, it } from "vitest"

import { toYaml } from "./toYaml"

describe("toYaml", () => {
  it("writes mappings, sequences and scalars in block style", () => {
    expect(
      toYaml({
        name: "send-email",
        attempts: 3,
        ok: false,
        parent: null,
        tags: ["a", "b"],
        opts: { backoff: { type: "exponential", delay: 1000 } },
      }),
    ).toBe(
      [
        "name: send-email",
        "attempts: 3",
        "ok: false",
        "parent: null",
        "tags:",
        "  - a",
        "  - b",
        "opts:",
        "  backoff:",
        "    type: exponential",
        "    delay: 1000",
        "",
      ].join("\n"),
    )
  })

  it("quotes strings YAML would read as something else", () => {
    expect(toYaml({ id: "1232", flag: "true", empty: "", dash: "- x", colon: "a: b" })).toBe(
      ['id: "1232"', 'flag: "true"', 'empty: ""', 'dash: "- x"', 'colon: "a: b"', ""].join("\n"),
    )
  })

  it("puts a mapping inside a sequence on the dash's line", () => {
    expect(toYaml({ items: [{ sku: "A1", qty: 2 }, []] })).toBe(
      ["items:", "  - sku: A1", "    qty: 2", "  - []", ""].join("\n"),
    )
  })

  it("writes multi-line strings as literal blocks", () => {
    expect(toYaml({ stack: "Error: boom\n    at worker.js:1" })).toBe(
      ["stack: |-", "  Error: boom", "      at worker.js:1", ""].join("\n"),
    )
  })

  it("empty containers and a bare scalar", () => {
    expect(toYaml({ a: {}, b: [] })).toBe("a: {}\nb: []\n")
    expect(toYaml(42)).toBe("42\n")
  })
})
