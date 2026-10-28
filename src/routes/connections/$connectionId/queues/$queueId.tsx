import { createFileRoute, Outlet } from "@tanstack/react-router"

/**
 * Queue-scoped layout route.
 *
 * Rendering the queue tabs directly, without an `<Outlet />`, would
 * leave the child `$jobId` route unmounted when the URL changes
 * (TanStack Router needs an Outlet in the parent): a click on a job
 * row would "do nothing visible" and the URL's `?view=` query param
 * would be lost, falling back to the route's default `view: overview`.
 *
 * So this route is layout-only: the tabbed content lives in the
 * sibling `./index.tsx`, and `$jobId.tsx` mounts via the Outlet
 * below.
 */
export const Route = createFileRoute("/connections/$connectionId/queues/$queueId")({
  component: QueueLayoutComponent,
})

function QueueLayoutComponent() {
  return <Outlet />
}
