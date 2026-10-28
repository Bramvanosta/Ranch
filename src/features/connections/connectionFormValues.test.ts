import { describe, expect, it } from "vitest"
import { bullmqConnection, redisEndpoint } from "@/domain/bullmqEndpoint"

import { aConnection } from "@/test/factories"

import type { ConnectionFormInput, ConnectionFormValues } from "./connectionFormSchema"
import {
  connectionFormDefaults,
  connectionSaveFields,
  testConnectionArgs,
} from "./connectionFormValues"

describe("connectionFormDefaults", () => {
  it("opens a new connection on a local Redis", () => {
    const values = connectionFormDefaults(null)
    expect(values).toMatchObject({ redis_host: "", redis_port: 6379, redis_db: 0, tls: false })
    expect(values.env_color).toBe("env-local")
  })

  it("fills an edited connection, passwords left blank", () => {
    const values = connectionFormDefaults(
      aConnection({
        name: "staging",
        ...bullmqConnection(redisEndpoint({ host: "redis.internal", port: 6380, username: "app" })),
      }),
    )
    expect(values).toMatchObject({
      name: "staging",
      redis_host: "redis.internal",
      redis_port: 6380,
      redis_username: "app",
      redis_password: "",
      ssh_password: "",
    })
  })
})

describe("testConnectionArgs", () => {
  const form = (patch: Partial<ConnectionFormInput>): ConnectionFormInput => ({
    ...connectionFormDefaults(null),
    redis_host: "localhost",
    ...patch,
  })

  it("sends blank fields as absent, and the CA only with TLS on", () => {
    expect(testConnectionArgs(form({ tls_custom_ca: "PEM" }), null)).toEqual({
      backend: "bullmq-redis",
      endpoint: redisEndpoint({ host: "localhost" }),
      password: null,
      sshPassword: null,
      savedConnectionId: null,
    })
    expect(testConnectionArgs(form({ tls: true, tls_custom_ca: "PEM" }), "c1")).toMatchObject({
      endpoint: { tls: true, tlsCustomCa: "PEM" },
      savedConnectionId: "c1",
    })
  })

  it("puts the SSH tunnel in the endpoint, its password beside it", () => {
    const args = testConnectionArgs(
      form({
        ssh_enabled: true,
        ssh_host: "bastion",
        ssh_user: "ops",
        ssh_auth_kind: "password",
        ssh_password: "secret",
      }),
      null,
    )
    expect(args.endpoint).toMatchObject({
      sshTunnel: { host: "bastion", port: 22, user: "ops", auth: { kind: "password" } },
    })
    expect(JSON.stringify(args.endpoint)).not.toContain("secret")
    expect(args.sshPassword).toBe("secret")
  })

  it("sends the SSH password only with password auth", () => {
    const ssh = { ssh_enabled: true, ssh_host: "bastion", ssh_password: "secret" }
    expect(testConnectionArgs(form({ ...ssh, ssh_auth_kind: "password" }), null).sshPassword).toBe(
      "secret",
    )
    expect(testConnectionArgs(form({ ...ssh, ssh_auth_kind: "keyFile" }), null).sshPassword).toBe(
      null,
    )
  })
})

describe("connectionSaveFields", () => {
  it("keeps an untouched lock for the backend to decide", () => {
    const values = {
      ...connectionFormDefaults(null),
      name: "local",
      redis_host: "localhost",
      environment_name: "Local",
      group_name: "Default",
    } as ConnectionFormValues
    const fields = connectionSaveFields(values, null)
    expect(fields).toMatchObject({
      name: "local",
      backend: "bullmq-redis",
      endpoint: { host: "localhost", port: 6379, db: 0 },
      password: null,
      locked: null,
    })
    expect(connectionSaveFields(values, true).locked).toBe(true)
  })
})
