import { createFileRoute, Outlet } from "@tanstack/react-router"
import { useConnections } from "@/data/connections/queries"
import { ConnectionCommands } from "@/features/connections"

/**
 * Connection-scoped layout: the connection's menu commands, and an
 * `<Outlet />` for its screens (the connection, a queue, a job). The URL
 * says which connection is selected (`useSelectedConnectionId`).
 */
export const Route = createFileRoute("/connections/$connectionId")({
  component: ConnectionLayoutComponent,
})

function ConnectionLayoutComponent() {
  const { connectionId } = Route.useParams()
  // Connection menu commands for this connection.
  const { data: connections } = useConnections()
  const connection = connections?.find((c) => c.id === connectionId)

  return (
    <>
      <Outlet />
      {connection ? <ConnectionCommands connection={connection} /> : null}
    </>
  )
}
