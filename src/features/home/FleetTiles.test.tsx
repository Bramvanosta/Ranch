/**
 * The fleet tiles never say "0 failed" before anything was read.
 */
import { render, screen, within } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({ commands: {} }))
vi.mock("@tanstack/react-router", () => ({ Link: () => null, useNavigate: () => vi.fn() }))

import { FleetTiles } from "./FleetTable"
import type { FleetRow } from "./useFleet"

const row = (over: Partial<FleetRow> = {}): FleetRow =>
  ({
    connection: { id: "c1", name: "prod" },
    health: undefined,
    error: null,
    errorAt: 0,
    workers: undefined,
    failedLastHour: undefined,
    latencyMs: undefined,
    latency: [],
    refetch: () => {},
    ...over,
  }) as unknown as FleetRow

const tile = (id: string) => within(screen.getByTestId(id))

describe("FleetTiles", () => {
  it("shows placeholders while the connections are being read", () => {
    render(
      <FleetTiles rows={[row(), row({ connection: { id: "c2" } as FleetRow["connection"] })]} />,
    )
    expect(tile("fleet-tile-failed").getByTestId("stat-tile-value")).toHaveTextContent("—")
    expect(tile("fleet-tile-failed").getByTestId("stat-tile-sub")).toHaveTextContent("loading")
    expect(tile("fleet-tile-waiting").getByTestId("stat-tile-value")).toHaveTextContent("—")
    expect(tile("fleet-tile-connections").getByTestId("stat-tile-sub")).toHaveTextContent(
      "checking",
    )
  })

  it("shows the figures once one connection answered", () => {
    const health = { failed: 3, waiting: 7, refs: [] } as unknown as FleetRow["health"]
    render(<FleetTiles rows={[row({ health, failedLastHour: 0 }), row()]} />)
    expect(tile("fleet-tile-failed").getByTestId("stat-tile-value")).toHaveTextContent("3")
    expect(tile("fleet-tile-waiting").getByTestId("stat-tile-value")).toHaveTextContent("7")
  })

  it("says 0 when every connection is down: nothing to wait for", () => {
    render(<FleetTiles rows={[row({ error: "refused" })]} />)
    expect(tile("fleet-tile-failed").getByTestId("stat-tile-value")).toHaveTextContent("0")
    expect(tile("fleet-tile-connections").getByTestId("stat-tile-sub")).toHaveTextContent(
      "1 offline",
    )
  })
})
