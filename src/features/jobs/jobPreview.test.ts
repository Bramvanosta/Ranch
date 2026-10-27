import { describe, expect, it } from "vitest"

import { attemptsLabel, oneLine, previewJobData, summarizeError } from "./jobPreview"

describe("previewJobData", () => {
  it("shows an object as `key: value` pairs", () => {
    expect(previewJobData('{"filmId":603,"source":"tmdb","force":true,"by":null}')).toBe(
      "filmId: 603 · source: tmdb · force: true · by: null",
    )
  })

  it("keeps nested values short", () => {
    expect(previewJobData('{"ids":[1,2,3],"user":{"id":1},"note":"a\\n  b"}')).toBe(
      "ids: [3] · user: {…} · note: a b",
    )
  })

  it("clips long strings and long payloads", () => {
    const sentence = "word ".repeat(20)
    expect(previewJobData(JSON.stringify({ a: sentence }))).toBe(`a: ${sentence.slice(0, 35)}…`)
    // No spaces: an identifier — its two ends.
    expect(previewJobData(JSON.stringify({ a: `${"x".repeat(40)}tail` }))).toBe("a: xxxxxxxx…tail")
    const many = Object.fromEntries(Array.from({ length: 40 }, (_, i) => [`key${i}`, i]))
    const preview = previewJobData(JSON.stringify(many), 60)
    expect(preview.length).toBe(60)
    expect(preview.endsWith("…")).toBe(true)
  })

  it("is empty for an empty or missing payload", () => {
    expect(previewJobData("{}")).toBe("")
    expect(previewJobData("[]")).toBe("")
    expect(previewJobData("null")).toBe("")
    expect(previewJobData(null)).toBe("")
  })

  it("shows what is not JSON as it is, and scalars as themselves", () => {
    expect(previewJobData("plain  text\nhere")).toBe("plain text here")
    expect(previewJobData("42")).toBe("42")
    expect(previewJobData('["a","b"]')).toBe("a, b")
  })
})

describe("previewJobData — what the row already shows", () => {
  it("leaves out a value the job id already contains, and shortens identifiers", () => {
    const uuid = "b83e828f-f9bb-4754-bb29-3c296744eaf8"
    const data = JSON.stringify({
      filmId: uuid,
      source: "TMDB",
      traceId: "0123456789abcdef0123456789abcdef",
    })
    expect(previewJobData(data, 160, `enrich-film-${uuid}`)).toBe(
      "source: TMDB · traceId: 01234567…cdef",
    )
    // Without the id, the uuid is kept — shortened in the middle.
    expect(previewJobData(data)).toContain("filmId: b83e828f…eaf8")
  })
})

describe("summarizeError", () => {
  const PRISMA = [
    "",
    "Invalid `tx.movie_production_countries.createMany()` invocation in",
    "/Users/me/app/src/film.prisma-repository.ts:416:45",
    "",
    "  413 await tx.movie_production_countries.deleteMany({ where: { movie_id: film.id } });",
    "  414 ",
    "  415 if (productionCountryCodes.length > 0) {",
    "→ 416   await tx.movie_production_countries.createMany(",
    "Unique constraint failed on the fields: (`movie_id`,`country_code`)",
  ].join("\n")

  it("a Prisma error: the message (its last line) first, then the call — no path, no code frame", () => {
    expect(summarizeError(PRISMA)).toBe(
      "Unique constraint failed on the fields: (`movie_id`,`country_code`) — tx.movie_production_countries.createMany()",
    )
  })

  it("a one-line error is left alone", () => {
    expect(summarizeError("connect ETIMEDOUT 10.0.4.12:5432")).toBe(
      "connect ETIMEDOUT 10.0.4.12:5432",
    )
  })

  it("a message followed by a stack keeps the message", () => {
    expect(
      summarizeError("Request failed with status code 503\n    at settle (/app/a.js:1:1)"),
    ).toBe("Request failed with status code 503")
  })

  it("any other multi-line error: first line — last line", () => {
    expect(summarizeError("ValidationError:\n  email: must be valid")).toBe(
      "ValidationError: — email: must be valid",
    )
  })
})

describe("oneLine", () => {
  it("collapses a multi-line failure reason that opens with a blank line", () => {
    expect(
      oneLine("\nInvalid `tx.movie.createMany()` invocation:\n\n\nUnique constraint failed"),
    ).toBe("Invalid `tx.movie.createMany()` invocation: Unique constraint failed")
  })
})

describe("attemptsLabel", () => {
  it("made / allowed, or made alone without a limit", () => {
    expect(attemptsLabel(3, '{"attempts":3}')).toBe("3/3")
    expect(attemptsLabel(1, "{}")).toBe("1")
    expect(attemptsLabel(2, "not json")).toBe("2")
    expect(attemptsLabel(0, null)).toBe("0")
  })
})
