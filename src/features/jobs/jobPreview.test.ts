import { describe, expect, it } from "vitest"

import { attemptsLabel, oneLine, summarizeError } from "./jobPreview"

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

  it("a Prisma error: the message (its last line) first, then the call, no path, no code frame", () => {
    expect(summarizeError(PRISMA)).toBe(
      "Unique constraint failed on the fields: (`movie_id`,`country_code`) (in tx.movie_production_countries.createMany())",
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

  it("any other multi-line error: first line / last line", () => {
    expect(summarizeError("ValidationError:\n  email: must be valid")).toBe(
      "ValidationError: / email: must be valid",
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
