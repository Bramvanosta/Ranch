import { describe, expect, it } from "vitest"

import { jobsSearch, queueSearchSchema } from "./queueSearch"

describe("queueSearchSchema", () => {
  it("opens on the jobs, and replaces a bad value with a safe one", () => {
    expect(queueSearchSchema.parse({})).toMatchObject({ view: "jobs", page: 0, fscope: "queue" })
    expect(queueSearchSchema.parse({ view: "nope", page: -3, size: 33 })).toMatchObject({
      view: "jobs",
      page: 0,
      size: undefined,
    })
  })
})

describe("jobsSearch", () => {
  it("keeps the list settings it does not change, on the first page", () => {
    const search = queueSearchSchema.parse({
      state: "failed",
      page: 4,
      fname: "invoice",
      fscope: "cross",
      sort: "duration",
      dir: "desc",
      size: 100,
    })
    expect(jobsSearch(search, { state: "waiting" })).toEqual({
      view: "jobs",
      state: "waiting",
      page: 0,
      fname: "invoice",
      ferror: undefined,
      fscope: "cross",
      sort: "duration",
      dir: "desc",
      size: 100,
      group: undefined,
    })
  })

  it("leaves the defaults out of the URL", () => {
    const search = queueSearchSchema.parse({ dir: "asc" })
    expect(jobsSearch(search, {})).toMatchObject({
      fname: undefined,
      fscope: undefined,
      dir: undefined,
    })
  })
})
