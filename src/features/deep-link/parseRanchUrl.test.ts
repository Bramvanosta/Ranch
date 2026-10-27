import { describe, expect, it } from "vitest"
import { parseNamedRanchUrl, parseRanchUrl, resolveNamedTarget } from "./parseRanchUrl"

describe("parseRanchUrl", () => {
  it("parses a connection-only URL", () => {
    expect(parseRanchUrl("ranch://abc-123")).toEqual({ connectionId: "abc-123" })
  })

  it("parses a connection + trailing slash", () => {
    expect(parseRanchUrl("ranch://abc-123/")).toEqual({ connectionId: "abc-123" })
  })

  it("parses a queue target", () => {
    expect(parseRanchUrl("ranch://conn-1/queues/emails")).toEqual({
      connectionId: "conn-1",
      queueName: "emails",
    })
  })

  it("parses a job target", () => {
    expect(parseRanchUrl("ranch://conn-1/queues/emails/jobs/42")).toEqual({
      connectionId: "conn-1",
      queueName: "emails",
      jobId: "42",
    })
  })

  it("decodes URL-encoded segments", () => {
    expect(parseRanchUrl("ranch://conn-1/queues/emails%20prod/jobs/job%2042")).toEqual({
      connectionId: "conn-1",
      queueName: "emails prod",
      jobId: "job 42",
    })
  })

  it("returns null for the wrong scheme", () => {
    expect(parseRanchUrl("https://example.com/whatever")).toBeNull()
  })

  it("returns null for a missing host", () => {
    expect(parseRanchUrl("ranch:///queues/emails")).toBeNull()
  })

  it("returns null for malformed paths", () => {
    expect(parseRanchUrl("ranch://conn-1/garbage")).toBeNull()
    expect(parseRanchUrl("ranch://conn-1/queues")).toBeNull()
    expect(parseRanchUrl("ranch://conn-1/queues/emails/jobs")).toBeNull()
    expect(parseRanchUrl("ranch://conn-1/queues/emails/jobs/42/extra")).toBeNull()
  })

  it("returns null on garbage input", () => {
    expect(parseRanchUrl("")).toBeNull()
    expect(parseRanchUrl("not a url")).toBeNull()
    expect(parseRanchUrl(null as unknown as string)).toBeNull()
  })
})

describe("named form — ranch://{group}/{environment}/…", () => {
  const connections = [
    { id: "c1", name: "queues", groupName: "Lumen", envLabel: "Production" },
    { id: "c2", name: "cache", groupName: "Lumen", envLabel: "Production" },
    { id: "c3", name: "local", groupName: "Lumen", envLabel: "Local" },
  ]

  it("is only reached when the id form does not match", () => {
    expect(parseRanchUrl("ranch://lumen/production")).toBeNull()
    expect(parseNamedRanchUrl("ranch://lumen/production")).toEqual({
      group: "lumen",
      environment: "production",
    })
  })

  it("parses queue, job, encoded names and the connection hint", () => {
    expect(
      parseNamedRanchUrl("ranch://my%20client/EU%20West/email-digest/42?connection=cache"),
    ).toEqual({
      group: "my client",
      environment: "EU West",
      queueName: "email-digest",
      jobId: "42",
      connectionName: "cache",
    })
  })

  it("rejects too few or too many segments", () => {
    expect(parseNamedRanchUrl("ranch://lumen")).toBeNull()
    expect(parseNamedRanchUrl("ranch://lumen/prod/q/1/extra")).toBeNull()
  })

  it("resolves case-insensitively to the first connection and flags ambiguity", () => {
    const r = resolveNamedTarget(
      { group: "lumen", environment: "PRODUCTION", queueName: "email" },
      connections,
    )
    expect(r).toEqual({
      ok: true,
      ambiguous: true,
      target: { connectionId: "c1", queueName: "email" },
    })
  })

  it("?connection= disambiguates", () => {
    const r = resolveNamedTarget(
      { group: "Lumen", environment: "Production", connectionName: "Cache" },
      connections,
    )
    expect(r).toEqual({ ok: true, ambiguous: false, target: { connectionId: "c2" } })
  })

  it("explains what is missing", () => {
    expect(resolveNamedTarget({ group: "acme", environment: "prod" }, connections)).toMatchObject({
      ok: false,
      reason: expect.stringContaining('group named "acme"'),
    })
    expect(
      resolveNamedTarget({ group: "lumen", environment: "staging" }, connections),
    ).toMatchObject({
      ok: false,
      reason: expect.stringContaining('no environment named "staging"'),
    })
  })
})
