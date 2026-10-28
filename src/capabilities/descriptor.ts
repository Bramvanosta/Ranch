import type {
  BackendInfo,
  CapabilityDescriptor,
  Connection,
  JobStateInfo,
} from "@/lib/tauri/bindings"

/**
 * One flag of a descriptor, by its path: the family and the variant
 * (`schedules.pause`), or the family alone when it is one flag (`flows`).
 * The same names an `unsupported` error gives.
 */
export type Need = {
  [F in Exclude<keyof CapabilityDescriptor, "jobStates">]: CapabilityDescriptor[F] extends boolean
    ? F
    : `${F}.${keyof CapabilityDescriptor[F] & string}`
}[Exclude<keyof CapabilityDescriptor, "jobStates">]

/** Whether `descriptor` declares `need`. Nothing is declared by an unknown descriptor. */
export function can(descriptor: CapabilityDescriptor | undefined, need: Need): boolean {
  if (!descriptor) return false
  const [family, variant] = need.split(".") as [keyof CapabilityDescriptor, string | undefined]
  const value = descriptor[family] as unknown
  if (variant === undefined) return value === true
  return (value as Record<string, unknown> | undefined)?.[variant] === true
}

/**
 * What every descriptor declares: a flag on in all of them, the states all
 * of them keep (in the first one's order). `undefined` for none.
 */
export function common(
  descriptors: ReadonlyArray<CapabilityDescriptor>,
): CapabilityDescriptor | undefined {
  const [first, ...rest] = descriptors
  if (!first) return undefined
  const both = (a: unknown, b: unknown): unknown =>
    typeof a === "boolean"
      ? a && b === true
      : Object.fromEntries(
          Object.entries(a as Record<string, unknown>).map(([k, v]) => [
            k,
            both(v, (b as Record<string, unknown>)[k]),
          ]),
        )
  return rest.reduce<CapabilityDescriptor>((acc, next) => {
    const { jobStates, ...flags } = acc
    const { jobStates: nextStates, ...nextFlags } = next
    const kept = new Set(nextStates.map((s) => s.code))
    return {
      ...(both(flags, nextFlags) as Omit<CapabilityDescriptor, "jobStates">),
      jobStates: jobStates.filter((s) => kept.has(s.code)),
    }
  }, first)
}

/**
 * The descriptor of a connection's backend, from the kinds listed and the
 * connection's `backend`. While the connection is not known yet, what
 * every kind here declares: with one kind, that kind's. A connection of a
 * kind this Ranch does not serve can do nothing.
 */
export function descriptorOf(
  backends: ReadonlyArray<BackendInfo> | undefined,
  connection: Pick<Connection, "backend"> | undefined,
): CapabilityDescriptor | undefined {
  if (!backends) return undefined
  if (!connection) return common(backends.map((b) => b.capabilities))
  return backends.find((b) => b.kind === connection.backend)?.capabilities
}

const NO_STATES: ReadonlyArray<JobStateInfo> = []

/** The states a descriptor describes; none while it is unknown. */
export function statesOf(
  descriptor: CapabilityDescriptor | undefined,
): ReadonlyArray<JobStateInfo> {
  return descriptor?.jobStates ?? NO_STATES
}
