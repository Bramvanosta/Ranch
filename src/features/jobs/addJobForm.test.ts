import { describe, expect, it } from "vitest"

import {
  type AddJobFormValues,
  addJobSnippet,
  describePlacement,
  EMPTY_ADD_JOB_FORM,
  formatJson,
  jsonError,
  validateAddJobForm,
} from "./addJobForm"

const form = (overrides: Partial<AddJobFormValues> = {}): AddJobFormValues => ({
  ...EMPTY_ADD_JOB_FORM,
  name: "send-email",
  data: '{"to":"a@b.c"}',
  ...overrides,
})

describe("backoff, job ID, keep when done / failed", () => {
  it("builds the options", () => {
    const result = validateAddJobForm(
      form({
        backoffKind: "exponential",
        backoffDelayMs: "2000",
        jobId: " order-1042 ",
        keepCompleted: false,
        keepFailed: true,
      }),
    )
    expect(result).toEqual({
      ok: true,
      job: expect.objectContaining({
        backoff: { kind: "exponential", delayMs: 2000 },
        jobId: "order-1042",
        removeOnComplete: true,
        removeOnFail: false,
      }),
    })
  })

  it("follows BullMQ's rules for a custom ID and asks for a backoff delay", () => {
    for (const [jobId, want] of [
      ["1042", "whole number"],
      ["a:b", "':'"],
    ] as const) {
      const result = validateAddJobForm(form({ jobId }))
      expect(result.ok).toBe(false)
      if (!result.ok) expect(result.errors.jobId).toContain(want)
    }
    const result = validateAddJobForm(form({ backoffKind: "fixed", backoffDelayMs: "" }))
    expect(result.ok ? null : result.errors.backoffDelayMs).toBe("A whole number of milliseconds")
  })

  it("writes the same job as code, with the options that were set", () => {
    expect(addJobSnippet(form())).toBe('await queue.add("send-email", {\n  "to": "a@b.c"\n})')
    expect(
      addJobSnippet(
        form({
          attempts: "3",
          backoffKind: "fixed",
          backoffDelayMs: "1000",
          jobId: "order-1",
          keepCompleted: false,
          lifo: true,
        }),
      ),
    ).toBe(
      [
        'await queue.add("send-email", {',
        '  "to": "a@b.c"',
        "}, {",
        "  attempts: 3,",
        '  backoff: { type: "fixed", delay: 1000 },',
        '  jobId: "order-1",',
        "  removeOnComplete: true,",
        "  lifo: true,",
        "})",
      ].join("\n"),
    )
  })
})

describe("validateAddJobForm", () => {
  it("builds the job: blanks mean 'not set'", () => {
    expect(validateAddJobForm(form({ name: "  send-email " }))).toEqual({
      ok: true,
      job: {
        name: "send-email",
        data: '{"to":"a@b.c"}',
        delayMs: 0,
        attempts: null,
        priority: 0,
        lifo: false,
        backoff: null,
        jobId: null,
        removeOnComplete: false,
        removeOnFail: false,
      },
    })
  })

  it("passes the options through as numbers", () => {
    const result = validateAddJobForm(
      form({ delayMs: " 60000 ", attempts: "3", priority: "5", lifo: true }),
    )
    expect(result).toEqual({
      ok: true,
      job: expect.objectContaining({ delayMs: 60000, attempts: 3, priority: 5, lifo: true }),
    })
  })

  it("reports every problem at once, next to its field", () => {
    const result = validateAddJobForm(
      form({ name: "a:b", data: "{oops", delayMs: "1.5", attempts: "0", priority: "-1" }),
    )
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(Object.keys(result.errors).sort()).toEqual([
      "attempts",
      "data",
      "delayMs",
      "name",
      "priority",
    ])
    expect(result.errors.name).toMatch(/cannot contain ':'/)
  })

  it("requires a name and a payload", () => {
    const result = validateAddJobForm(form({ name: " ", data: "  " }))
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.errors.name).toMatch(/name/)
    expect(result.errors.data).toMatch(/use \{\}/)
  })

  it("accepts any JSON value as payload", () => {
    for (const data of ["null", "[1,2]", '"text"', "42", "{}"]) {
      expect(validateAddJobForm(form({ data })).ok).toBe(true)
    }
  })
})

describe("json helpers", () => {
  it("jsonError is null for valid JSON and a message otherwise", () => {
    expect(jsonError('{"a":1}')).toBeNull()
    expect(jsonError("{a:1}")).toEqual(expect.any(String))
  })

  it("formatJson pretty-prints valid JSON and leaves the rest alone", () => {
    expect(formatJson('{"a":[1,2]}')).toBe('{\n  "a": [\n    1,\n    2\n  ]\n}')
    expect(formatJson("{oops")).toBe("{oops")
  })
})

describe("describePlacement", () => {
  it("follows BullMQ's precedence: delay, then priority, then the wait list", () => {
    expect(describePlacement(form({ delayMs: "500", priority: "2" }), false)).toMatch(/delayed/)
    expect(describePlacement(form({ priority: "2" }), false)).toMatch(/prioritized/)
    expect(describePlacement(form(), false)).toMatch(/behind the jobs/)
    expect(describePlacement(form({ lifo: true }), false)).toMatch(/front of waiting/)
  })

  it("says so when the queue is paused", () => {
    expect(describePlacement(form(), true)).toMatch(/paused until it is resumed/)
    expect(describePlacement(form({ priority: "2" }), true)).toMatch(/waits for a resume/)
  })
})
