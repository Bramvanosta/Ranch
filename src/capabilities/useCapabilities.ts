import { useCallback, useMemo } from "react"

import { useBackends, useConnection, useConnections } from "@/data/connections/queries"
import type { CapabilityDescriptor, JobStateInfo } from "@/lib/tauri/bindings"

import { can, descriptorOf, type Need, statesOf } from "./descriptor"

/** What a connection's backend can do, as a screen asks it. */
export interface Capabilities {
  /** `undefined` until the kinds of backend are listed: then nothing is capable. */
  descriptor: CapabilityDescriptor | undefined
  can: (need: Need) => boolean
  /** The job states the backend keeps, in the order it gives them. */
  jobStates: ReadonlyArray<JobStateInfo>
}

function capabilities(descriptor: CapabilityDescriptor | undefined): Capabilities {
  return {
    descriptor,
    can: (need) => can(descriptor, need),
    jobStates: statesOf(descriptor),
  }
}

/**
 * What the backend of `connectionId` can do: its kind's descriptor, from
 * the kinds `list_backends` lists (read once, they do not change while
 * Ranch runs) and the connection's `backend`.
 */
export function useCapabilities(connectionId: string | null | undefined): Capabilities {
  const { data: backends } = useBackends()
  const connection = useConnection(connectionId)
  return useMemo(() => capabilities(descriptorOf(backends, connection)), [backends, connection])
}

/** The same for any connection, for a screen that lists several. */
export function useCapabilityLookup(): (connectionId: string) => Capabilities {
  const { data: backends } = useBackends()
  const { data: connections } = useConnections()
  return useCallback(
    (connectionId) =>
      capabilities(
        descriptorOf(
          backends,
          connections?.find((c) => c.id === connectionId),
        ),
      ),
    [backends, connections],
  )
}

/** The job states the backend of `connectionId` describes: code, label, tone. */
export function useJobStates(connectionId: string | null | undefined): ReadonlyArray<JobStateInfo> {
  return useCapabilities(connectionId).jobStates
}
