import { useQuery } from "@tanstack/react-query"
import { createFileRoute, Outlet } from "@tanstack/react-router"
import { useEffect } from "react"

import { ConnectionCommands } from "@/features/connections/ConnectionCommands"
import { commands } from "@/lib/tauri/bindings"
import { useSelectionStore } from "@/stores/useSelectionStore"

/**
 * Connection-scoped layout route.
 *
 * Stories that built up to this view:
 *   - 1.5: route placeholder + selection-store mirror.
 *   - 2.3: queue list lived here directly.
 *   - 3.1 polish: split the queue list into a sibling index child
 *     (`./index.tsx`) so this route can be a pure layout with an
 *     `<Outlet />` for nested routes (queue detail, job detail).
 *     Without this, clicking a queue Link changed the URL but the
 *     child never rendered — the parent component owned the canvas.
 *
 * The mirror into `useSelectionStore` is preserved so non-route
 * components (status bar, future breadcrumb) can read the active
 * selection without router context. `usePersistSelection` (mounted
 * at the root) writes the change through to `kv_settings`.
 */
export const Route = createFileRoute("/connections/$connectionId")({
  component: ConnectionLayoutComponent,
})

function ConnectionLayoutComponent() {
  const { connectionId } = Route.useParams()
  const select = useSelectionStore((s) => s.select)

  useEffect(() => {
    select(connectionId)
    return () => {
      select(null)
    }
  }, [connectionId, select])

  // Connection menu commands for this connection.
  const { data: connections } = useQuery({
    queryKey: ["connections"],
    queryFn: async () => {
      const r = await commands.listConnections()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
  })
  const connection = connections?.find((c) => c.id === connectionId)

  return (
    <>
      <Outlet />
      {connection ? <ConnectionCommands connection={connection} /> : null}
    </>
  )
}
