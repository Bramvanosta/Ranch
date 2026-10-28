import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { type RenderOptions, render, renderHook } from "@testing-library/react"
import type { ReactElement, ReactNode } from "react"

import { BULLMQ_REDIS_INFO } from "@/backend/fake/backends"
import { TooltipProvider } from "@/components/ui/tooltip"
import { connectionKeys } from "@/data/connections/keys"
import type { BackendInfo } from "@/lib/tauri/bindings"

/**
 * The providers every screen expects: a fresh query client (no retries, so
 * an error shows at once) and the tooltip provider. Each call gets its own
 * client, so no cache leaks between tests.
 *
 * The client already holds the kinds of backend (`list_backends`): BullMQ
 * on Redis, as the app lists it, so screens offer what BullMQ can do. A
 * test of another kind passes its own.
 */
export function testQueryClient(backends: BackendInfo[] = [BULLMQ_REDIS_INFO]) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  client.setQueryData(connectionKeys.backends, backends)
  return client
}

export function providers(client = testQueryClient()) {
  function Providers({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={client}>
        <TooltipProvider>{children}</TooltipProvider>
      </QueryClientProvider>
    )
  }
  return Providers
}

export function renderWithProviders(
  ui: ReactElement,
  { client = testQueryClient(), ...options }: { client?: QueryClient } & RenderOptions = {},
) {
  return { client, ...render(ui, { wrapper: providers(client), ...options }) }
}

export function renderHookWithProviders<T>(hook: () => T, client = testQueryClient()) {
  return { client, ...renderHook(hook, { wrapper: providers(client) }) }
}
