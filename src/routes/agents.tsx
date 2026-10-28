/**
 * The Agents page: `?tab=approvals|activity|notes|setup`.
 */
import { createFileRoute, useNavigate } from "@tanstack/react-router"
import { z } from "zod"

import { AGENTS_TABS, AgentsPage } from "@/features/agents"

export const Route = createFileRoute("/agents")({
  component: AgentsRoute,
  validateSearch: z.object({
    tab: z.enum(AGENTS_TABS).default("approvals").catch("approvals"),
  }),
})

function AgentsRoute() {
  const { tab } = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  return (
    <AgentsPage
      tab={tab}
      onTab={(next) => void navigate({ search: { tab: next }, replace: true })}
    />
  )
}
