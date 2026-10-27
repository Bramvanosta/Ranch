import { useQuery } from "@tanstack/react-query"

import { commands, type QueueRef } from "@/lib/tauri/bindings"

function unwrap<T>(
  r: { status: "ok"; data: T } | { status: "error"; error: { kind: string; message: string } },
): T {
  if (r.status === "error") {
    const err = new Error(r.error.message)
    ;(err as Error & { kind?: string }).kind = r.error.kind
    throw err
  }
  return r.data
}

/**
 * The flow a job belongs to. Every job has one — a job
 * that is in no flow comes back as a tree of one node — so callers decide
 * what to show from `nodes.length`.
 */
export function useFlow(args: { connectionId: string; queueRef: QueueRef; jobId: string }) {
  return useQuery({
    queryKey: ["flow", args.connectionId, args.queueRef.prefix, args.queueRef.name, args.jobId],
    queryFn: async () => unwrap(await commands.getFlow(args)),
    staleTime: 10_000,
    retry: 1,
  })
}

/** Flow roots found in a queue (bounded discovery). */
export function useFlowRoots(args: { connectionId: string; queueRef: QueueRef }) {
  return useQuery({
    queryKey: ["flowRoots", args.connectionId, args.queueRef.prefix, args.queueRef.name],
    queryFn: async () => unwrap(await commands.listFlowRoots(args)),
    staleTime: 10_000,
    retry: 1,
  })
}
