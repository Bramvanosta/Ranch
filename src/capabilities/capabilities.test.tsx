import { screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { BULLMQ_CAPABILITIES, BULLMQ_REDIS_INFO } from "@/backend/fake/backends"
import { StatePill } from "@/components/data/StatePill"
import { connectionKeys } from "@/data/connections/keys"
import type { BackendInfo, CapabilityDescriptor } from "@/lib/tauri/bindings"
import { aConnection } from "@/test/factories"
import {
  renderHookWithProviders,
  renderWithProviders,
  testQueryClient,
} from "@/test/renderWithProviders"

import { can, common, descriptorOf } from "./descriptor"
import { IfCapable, useCapabilities, useJobStates } from "./index"

/** A kind that keeps jobs and parks them, and does nothing with schedulers. */
const PARKING: CapabilityDescriptor = {
  ...BULLMQ_CAPABILITIES,
  jobStates: [
    { code: "waiting", label: "Waiting", tone: "pending", common: true },
    { code: "failed", label: "Failed", tone: "danger", common: true },
    { code: "parked", label: "Parked here", tone: "warning", common: false },
  ],
  schedules: { list: false, write: false, pause: false },
  queueControl: false,
}
const PARKING_INFO: BackendInfo = { ...BULLMQ_REDIS_INFO, kind: "parking", capabilities: PARKING }

describe("a descriptor", () => {
  it("declares a flag by its path", () => {
    expect(can(BULLMQ_CAPABILITIES, "schedules.pause")).toBe(true)
    expect(can(BULLMQ_CAPABILITIES, "flows")).toBe(true)
    expect(can(PARKING, "schedules.list")).toBe(false)
    expect(can(PARKING, "queueControl")).toBe(false)
    expect(can(undefined, "flows")).toBe(false)
  })

  it("is a connection's kind's, or what every kind declares while the connection is not known", () => {
    const both = [BULLMQ_REDIS_INFO, PARKING_INFO]
    expect(descriptorOf(both, { backend: "parking" })).toBe(PARKING)
    expect(descriptorOf(both, { backend: "elsewhere" })).toBeUndefined()
    expect(descriptorOf(undefined, { backend: "parking" })).toBeUndefined()
    expect(descriptorOf([BULLMQ_REDIS_INFO], undefined)).toEqual(BULLMQ_CAPABILITIES)

    const shared = common([BULLMQ_CAPABILITIES, PARKING])
    expect(shared?.jobStates.map((s) => s.code)).toEqual(["waiting", "failed"])
    expect(can(shared, "jobActions.single")).toBe(true)
    expect(can(shared, "schedules.list")).toBe(false)
  })
})

describe("useCapabilities", () => {
  function clientWith(connection: ReturnType<typeof aConnection>) {
    const client = testQueryClient([BULLMQ_REDIS_INFO, PARKING_INFO])
    client.setQueryData(connectionKeys.all, [connection])
    return client
  }

  it("answers for the connection's kind, its states described", () => {
    const parked = aConnection({ id: "p1", backend: "parking" })
    const { result } = renderHookWithProviders(
      () => ({ caps: useCapabilities("p1"), states: useJobStates("p1") }),
      clientWith(parked),
    )
    expect(result.current.caps.can("schedules.list")).toBe(false)
    expect(result.current.caps.can("jobActions.single")).toBe(true)
    expect(result.current.states.map((s) => [s.code, s.label, s.tone])).toContainEqual([
      "parked",
      "Parked here",
      "warning",
    ])
  })

  it("offers nothing until the kinds are listed", () => {
    const client = testQueryClient()
    client.removeQueries({ queryKey: connectionKeys.backends })
    client.setQueryDefaults(connectionKeys.backends, { enabled: false })
    const { result } = renderHookWithProviders(() => useCapabilities("c1"), client)
    expect(result.current.descriptor).toBeUndefined()
    expect(result.current.can("jobActions.single")).toBe(false)
    expect(result.current.jobStates).toEqual([])
  })

  it("gives a state of the kind's own its label and tone on a pill", () => {
    function Pill() {
      return <StatePill state="parked" states={useJobStates("p1")} />
    }
    renderWithProviders(<Pill />, {
      client: clientWith(aConnection({ id: "p1", backend: "parking" })),
    })
    const pill = screen.getByTitle("State: parked here")
    expect(pill).toHaveTextContent("parked here")
    expect(pill.className).toContain("text-(--state-delayed-text)")
  })

  it("shows what is declared and nothing else", () => {
    renderWithProviders(
      <>
        <IfCapable connectionId="p1" need="jobActions.single">
          <button type="button">Retry</button>
        </IfCapable>
        <IfCapable connectionId="p1" need="schedules.write">
          <button type="button">New scheduler</button>
        </IfCapable>
      </>,
      { client: clientWith(aConnection({ id: "p1", backend: "parking" })) },
    )
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "New scheduler" })).not.toBeInTheDocument()
  })
})
