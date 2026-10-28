import { useRouterState } from "@tanstack/react-router"

/** `/connections/c1/queues/emails` → `c1`; any other path → `null`. */
export function connectionIdOfPath(pathname: string): string | null {
  const segment = /^\/connections\/([^/?#]+)/.exec(pathname)?.[1]
  return segment === undefined ? null : decodeURIComponent(segment)
}

/**
 * The connection on screen, read from the URL: the route is the only place
 * that says which connection is selected, so nothing has to mirror it.
 */
export function useSelectedConnectionId(): string | null {
  return useRouterState({ select: (s) => connectionIdOfPath(s.location.pathname) })
}
