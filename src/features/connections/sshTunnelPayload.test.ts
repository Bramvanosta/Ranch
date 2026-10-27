import { describe, expect, it } from "vitest"

import type { Connection } from "@/lib/tauri/bindings"

import type { ConnectionFormInput } from "./connectionFormSchema"
import { buildSshTunnelPayload, parseSshTunnelDefaults } from "./sshTunnelPayload"

function input(overrides: Partial<ConnectionFormInput> = {}): ConnectionFormInput {
  return {
    name: "n",
    redis_host: "h",
    redis_port: 6379,
    redis_db: 0,
    redis_username: "",
    redis_password: "",
    tls: false,
    tls_custom_ca: "",
    ssh_enabled: false,
    ssh_host: "",
    ssh_port: 22,
    ssh_user: "",
    ssh_auth_kind: "keyFile",
    ssh_key_file_path: "",
    ssh_password: "",
    ssh_jump_enabled: false,
    ssh_jump_host: "",
    ssh_jump_port: 22,
    ssh_jump_user: "",
    env_color: "env-local",
    ...overrides,
  }
}

describe("buildSshTunnelPayload", () => {
  it("returns null when ssh_enabled is false", () => {
    expect(buildSshTunnelPayload(input({ ssh_enabled: false }))).toBeNull()
  })

  it("returns null even if ssh_* fields are filled but toggle is off (defensive)", () => {
    expect(
      buildSshTunnelPayload(
        input({
          ssh_enabled: false,
          ssh_host: "bastion",
          ssh_user: "ubuntu",
          ssh_key_file_path: "/k",
        }),
      ),
    ).toBeNull()
  })

  it("builds the key-file variant", () => {
    expect(
      buildSshTunnelPayload(
        input({
          ssh_enabled: true,
          ssh_host: "bastion.example.com",
          ssh_port: 2222,
          ssh_user: "ubuntu",
          ssh_auth_kind: "keyFile",
          ssh_key_file_path: "/Users/alex/.ssh/id_ed25519",
        }),
      ),
    ).toEqual({
      host: "bastion.example.com",
      port: 2222,
      user: "ubuntu",
      auth: { kind: "keyFile", keyFilePath: "/Users/alex/.ssh/id_ed25519" },
      jumpHost: null,
    })
  })

  it("builds the password variant", () => {
    expect(
      buildSshTunnelPayload(
        input({
          ssh_enabled: true,
          ssh_host: "h",
          ssh_user: "u",
          ssh_auth_kind: "password",
        }),
      ),
    ).toEqual({
      host: "h",
      port: 22,
      user: "u",
      auth: { kind: "password" },
      jumpHost: null,
    })
  })

  it("includes jump host when ssh_jump_enabled", () => {
    expect(
      buildSshTunnelPayload(
        input({
          ssh_enabled: true,
          ssh_host: "h",
          ssh_user: "u",
          ssh_jump_enabled: true,
          ssh_jump_host: "jump",
          ssh_jump_port: 2200,
          ssh_jump_user: "j",
        }),
      ),
    ).toMatchObject({
      jumpHost: { host: "jump", port: 2200, user: "j" },
    })
  })
})

describe("parseSshTunnelDefaults", () => {
  it("returns the no-SSH baseline when connection is null", () => {
    expect(parseSshTunnelDefaults(null)).toMatchObject({
      ssh_enabled: false,
      ssh_port: 22,
      ssh_jump_port: 22,
      ssh_auth_kind: "keyFile",
    })
  })

  it("maps a key-file connection back to flat form values", () => {
    const conn = {
      sshTunnel: {
        host: "bastion.example.com",
        port: 2222,
        user: "ubuntu",
        auth: { kind: "keyFile", keyFilePath: "/k" },
        jumpHost: null,
      },
    } as Partial<Connection> as Connection
    expect(parseSshTunnelDefaults(conn)).toEqual({
      ssh_enabled: true,
      ssh_host: "bastion.example.com",
      ssh_port: 2222,
      ssh_user: "ubuntu",
      ssh_auth_kind: "keyFile",
      ssh_key_file_path: "/k",
      ssh_jump_enabled: false,
      ssh_jump_host: "",
      ssh_jump_port: 22,
      ssh_jump_user: "",
    })
  })

  it("maps a password+jump-host connection back to flat form values", () => {
    const conn = {
      sshTunnel: {
        host: "b",
        port: 22,
        user: "u",
        auth: { kind: "password" },
        jumpHost: { host: "j", port: 22, user: "ju" },
      },
    } as Partial<Connection> as Connection
    expect(parseSshTunnelDefaults(conn)).toMatchObject({
      ssh_enabled: true,
      ssh_auth_kind: "password",
      ssh_key_file_path: "",
      ssh_jump_enabled: true,
      ssh_jump_host: "j",
      ssh_jump_user: "ju",
    })
  })
})
