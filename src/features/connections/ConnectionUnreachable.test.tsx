import { fireEvent, render, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"
import { bullmqConnection, redisEndpoint } from "@/domain/bullmqEndpoint"

// The edit dialog is its own component with its own tests: here it only has
// to open with the right connection.
vi.mock("./EditConnectionDialog", () => ({
  EditConnectionDialog: ({ open, connection }: { open: boolean; connection: { name: string } }) =>
    open ? <div data-testid="edit-dialog">{connection.name}</div> : null,
}))
const refresh = { auto: true }
vi.mock("@/data/settings/useRefreshSettings", () => ({
  useRefreshSettings: () => ({ settings: refresh }),
}))

import type { Connection } from "@/lib/tauri/bindings"

import { ConnectionUnreachable } from "./ConnectionUnreachable"

const connection = {
  id: "c1",
  name: "playground",
  ...bullmqConnection(redisEndpoint({ host: "localhost", port: 6380, db: 2 })),
} as Connection

describe("ConnectionUnreachable", () => {
  it("says which Redis, what the server answered, and what usually causes it", () => {
    render(
      <ConnectionUnreachable
        connection={connection}
        message="Connection refused (os error 61)"
        kind="redis"
        onRetry={() => {}}
        retrying={false}
      />,
    )
    const screenEl = screen.getByTestId("connection-unreachable")
    expect(screenEl).toHaveTextContent("Can't reach playground")
    expect(screenEl).toHaveTextContent("localhost:6380 · db 2")
    expect(screen.getByTestId("connection-unreachable-message")).toHaveTextContent(
      "Connection refused (os error 61)",
    )
    expect(screenEl).toHaveTextContent("Is Redis running")
    expect(screenEl).toHaveTextContent("keeps trying in the background")
  })

  it("tunes the hints to the failure (SSH tunnel)", () => {
    render(
      <ConnectionUnreachable
        connection={connection}
        message="tunnel: timed out"
        kind="sshTunnelFailed"
        onRetry={() => {}}
        retrying={false}
      />,
    )
    expect(screen.getByTestId("connection-unreachable")).toHaveTextContent("SSH tunnel")
    expect(screen.getByTestId("connection-unreachable")).not.toHaveTextContent("Is Redis running")
  })

  it("retries on demand, also while a background poll is in flight", () => {
    const onRetry = vi.fn()
    render(
      <ConnectionUnreachable
        connection={connection}
        message="Connection refused"
        onRetry={onRetry}
        retrying
      />,
    )
    const retry = screen.getByTestId("connection-unreachable-retry")
    expect(retry).toBeEnabled()
    fireEvent.click(retry)
    expect(onRetry).toHaveBeenCalledTimes(1)
  })

  it("opens the connection's settings", () => {
    render(
      <ConnectionUnreachable
        connection={connection}
        message="WRONGPASS"
        onRetry={() => {}}
        retrying={false}
      />,
    )
    expect(screen.queryByTestId("edit-dialog")).not.toBeInTheDocument()
    fireEvent.click(screen.getByTestId("connection-unreachable-edit"))
    expect(screen.getByTestId("edit-dialog")).toHaveTextContent("playground")
  })

  it("does not promise a comeback when auto-refresh is off", () => {
    refresh.auto = false
    render(
      <ConnectionUnreachable
        connection={connection}
        message="Connection refused"
        onRetry={() => {}}
        retrying={false}
      />,
    )
    expect(screen.getByTestId("connection-unreachable")).not.toHaveTextContent("keeps trying")
    refresh.auto = true
  })
})
