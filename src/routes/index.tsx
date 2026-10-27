import { createFileRoute } from "@tanstack/react-router"

import { HomePage } from "@/features/home/HomePage"

/**
 * Landing route at `/` — the home page: every connection as a
 * card, grouped, with live health.
 *
 * This route does not redirect to the last-selected connection (that made it a
 * blank placeholder nobody ever saw). The home page is the landing page; the last-opened connection is marked on its card instead
 * (one click away), and deep links still open their target directly.
 */
export const Route = createFileRoute("/")({
  component: HomePage,
})
