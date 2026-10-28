import { describe, expect, it } from "vitest"

import { checkJson } from "./jsonCheck"

describe("checkJson", () => {
  it("summarises a valid payload", () => {
    expect(checkJson('{\n  "to": "a@b.c",\n  "n": 1,\n  "x": [1, 2],\n  "y": null\n}')).toEqual({
      ok: true,
      summary: "Valid JSON · 4 keys · 39 bytes",
    })
    expect(checkJson("[1, 2, 3]")).toMatchObject({ summary: "Valid JSON · 3 items · 7 bytes" })
    expect(checkJson('"text"')).toMatchObject({ summary: "Valid JSON · string · 6 bytes" })
  })

  it("counts the bytes Redis will hold: whitespace outside strings dropped", () => {
    expect(checkJson('{ "a b": "x  y" }')).toMatchObject({
      summary: "Valid JSON · 1 key · 14 bytes",
    })
  })

  it("points at the line of the error", () => {
    expect(checkJson('{\n  "to": "a@b.c",\n  "n": 1,\n}')).toEqual({
      ok: false,
      message: "No comma after the last property in JSON",
      line: 4,
      column: 1,
    })
    expect(checkJson('{\n  "to": "a@b.c"\n  "n": 1\n}')).toMatchObject({
      message: "Expected ',' or '}' after the value",
      line: 3,
    })
    expect(checkJson("{\n  'to': 1\n}")).toMatchObject({ line: 2, column: 3 })
    expect(checkJson('{"a": tru}')).toMatchObject({ message: "Expected a value, found 't'" })
    expect(checkJson('{"a": 1')).toMatchObject({
      message: "The object is not closed: '}' is missing",
    })
    expect(checkJson('{"a": "x')).toMatchObject({ line: 1, column: 7 })
    expect(checkJson("{} {}")).toMatchObject({ message: "Unexpected text after the value" })
  })

  it("an empty payload is required", () => {
    expect(checkJson("  ")).toMatchObject({ ok: false, line: 1 })
  })

  it("agrees with JSON.parse on what is valid", () => {
    for (const text of [
      '{"a":"\\u00e9\\n"}',
      "-0.5e10",
      "[[], {}]",
      "true",
      '{"a":{"b":[null]}}',
    ]) {
      expect(checkJson(text).ok).toBe(true)
      expect(() => JSON.parse(text)).not.toThrow()
    }
    for (const text of ["01", "[1,]", '{"a" 1}', "nul", '"\\x"', "1.", ".5"]) {
      expect(checkJson(text).ok).toBe(false)
      expect(() => JSON.parse(text)).toThrow()
    }
  })
})
