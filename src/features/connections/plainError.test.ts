import { describe, expect, it } from "vitest"

import { bullmqConnection, redisEndpoint } from "@/domain/bullmqEndpoint"

import { plainConnectionError, plainTestError } from "./plainError"

const c = bullmqConnection(redisEndpoint({ host: "redis.local", port: 6380 }))

describe("plainConnectionError", () => {
  it("says what went wrong and what to check", () => {
    expect(plainConnectionError("Connection refused (os error 61)", c)).toBe(
      "Connection refused. Is Redis running on port 6380?",
    )
    expect(plainConnectionError("connection timed out", c)).toBe(
      "No answer from redis.local:6380. Is the host reachable (VPN, firewall)?",
    )
    expect(plainConnectionError("WRONGPASS invalid username-password pair", c)).toContain(
      "password",
    )
    expect(
      plainConnectionError(
        "failed to lookup address information: nodename nor servname provided",
        c,
      ),
    ).toBe("The host name redis.local does not resolve.")
    expect(plainConnectionError("invalid peer certificate: UnknownIssuer", c)).toContain("TLS")
    expect(plainConnectionError("something odd", c)).toBe("something odd")
  })

  it("names no host for an endpoint that is not a Redis one", () => {
    const other = { backend: "memory", endpoint: {} }
    expect(plainConnectionError("Connection refused", other)).toBe("Connection refused.")
    expect(plainConnectionError("connection timed out", other)).toBe(
      "No answer from the server. Is the host reachable (VPN, firewall)?",
    )
  })
})

describe("plainTestError", () => {
  const c = { host: "redis.local", port: 6390 }

  it("explains each kind of failure in plain words", () => {
    expect(plainTestError({ kind: "authFailed" }, c).headline).toMatch(
      /refused the user or password/,
    )
    expect(plainTestError({ kind: "timeout" }, c).headline).toBe(
      "No answer from redis.local:6390 within 4 seconds. Is the host reachable (VPN, firewall)?",
    )
    expect(plainTestError(null, c).headline).toMatch(/without saying why/)
  })

  it("keeps the server's words only when they add something", () => {
    expect(plainTestError({ kind: "cannotReach", reason: "Connection refused" }, c)).toEqual({
      headline: "Connection refused. Is Redis running on port 6390?",
      detail: null,
    })
    expect(
      plainTestError({ kind: "cannotReach", reason: "Connection refused (os error 61)" }, c).detail,
    ).toBe("Connection refused (os error 61)")
    expect(plainTestError({ kind: "other", reason: "weird" }, c)).toEqual({
      headline: "weird",
      detail: null,
    })
    expect(plainTestError({ kind: "sshTunnel", reason: "bad key" }, c).detail).toBe("bad key")
  })
})
