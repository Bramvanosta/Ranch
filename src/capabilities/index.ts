/**
 * What a connection's backend can do, as screens follow it. Each kind of
 * backend declares its capabilities and the job states it keeps once, and
 * `list_backends` lists them with the kind (`BackendInfo.capabilities`); a
 * connection's are its kind's (`connection.backend`). Screens ask here and
 * nowhere else:
 *
 * - `useCapabilities(connectionId).can("schedules.pause")`, or
 *   `<IfCapable connectionId need="schedules.write">`: an action, a menu
 *   item or a tab the backend does not declare is absent, not disabled.
 * - `useJobStates(connectionId)`: the states it keeps, each with its code,
 *   label and tone, for pills, state tabs and totals
 *   (`src/domain/jobStates.ts` has the pure helpers).
 *
 * Until the kinds are listed, nothing is capable: an action appears once
 * known and never shows before disappearing. While the connection itself
 * is not known yet, it can do what every kind here declares, which with
 * one kind is that kind's. Hiding is a convenience: the core refuses what
 * a backend does not declare, `unsupported` naming it, whatever the UI
 * shows.
 *
 * These are the backend's capabilities, known without connecting. What
 * the runtime offers (native windows, live events, the updater) is
 * another axis, asked of `src/lib/runtime/runtime.ts`; what one server
 * refuses at run time (a Redis without `CLIENT LIST`) is the call's error
 * or empty state.
 */

export { can, type Need } from "./descriptor"
export { IfCapable } from "./IfCapable"
export {
  type Capabilities,
  useCapabilities,
  useCapabilityLookup,
  useJobStates,
} from "./useCapabilities"
