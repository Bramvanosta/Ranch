import { createFileRoute } from "@tanstack/react-router"

import { AttentionPage } from "@/features/attention"

/** Everything that needs the user, across connections. */
export const Route = createFileRoute("/attention")({
  component: AttentionPage,
})
