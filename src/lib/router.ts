import { createRouter } from "@tanstack/react-router"

import { RouteError, RouteNotFound } from "@/components/RouteError"
import { routeTree } from "@/routeTree.gen"

/**
 * TanStack Router instance.
 *
 * The route tree is generated at build time by `@tanstack/router-plugin/vite`
 * from the files under `src/routes/`. Add a route = add a file; the plugin
 * regenerates `routeTree.gen.ts` on the next dev server cycle.
 *
 * The `routeTree.gen.ts` file is committed to git (like `bindings.ts`) so
 * CI/release builds skip the codegen step.
 */
export const router = createRouter({
  routeTree,
  // A page that throws, or a URL that matches nothing, says so inside the
  // shell — not with the router's stock full-window dump.
  defaultErrorComponent: RouteError,
  defaultNotFoundComponent: RouteNotFound,
})

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router
  }
}
