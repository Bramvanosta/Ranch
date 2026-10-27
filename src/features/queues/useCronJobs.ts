import { useQuery } from "@tanstack/react-query"

import { commands, type QueueRef } from "@/lib/tauri/bindings"

/**
 * TanStack Query hook for the CRON / repeatable jobs view.
 *
 * Hits `list_cron_jobs` which reads the `bull:{queue}:repeat` ZSET +
 * pipelines HGETALL per scheduler. Cached 30s; `schemaNotRecognized`
 * surfaces immediately without retry (same contract as `useQueues`/
 * `useJobs`/`useJob`); transient failures retry up to 2x.
 */
export interface UseCronJobsArgs {
  connectionId: string
  queueRef: QueueRef
}

export function useCronJobs(args: UseCronJobsArgs) {
  return useQuery({
    queryKey: ["cronJobs", args.connectionId, args.queueRef.prefix, args.queueRef.name],
    queryFn: async () => {
      const result = await commands.listCronJobs({
        connectionId: args.connectionId,
        queueRef: args.queueRef,
      })
      if (result.status === "error") {
        const err = new Error(result.error.message)
        ;(err as Error & { kind?: string }).kind = result.error.kind
        throw err
      }
      return result.data
    },
    staleTime: 30_000,
    retry: (failureCount, error) => {
      const kind = (error as Error & { kind?: string }).kind
      if (kind === "schemaNotRecognized") return false
      return failureCount < 2
    },
  })
}
