/**
 * Listens for `ranch://` deep-link events and dispatches
 * navigation through TanStack Router.
 *
 * The Tauri deep-link plugin emits `onOpenUrl` with the raw URL the
 * OS handed off. We parse it, validate the connection exists, then
 * route to the deepest valid segment.
 *
 * Unknown-id graceful path: if the connection isn't in the
 * user's saved list, the user lands on the empty sidebar root with
 * a toast explaining the missing target — no crash, no blank page.
 */

import { useNavigate } from "@tanstack/react-router"
import { onOpenUrl } from "@tauri-apps/plugin-deep-link"
import { useEffect } from "react"
import { toast } from "sonner"

import { commands } from "@/lib/tauri/bindings"

import {
  parseNamedRanchUrl,
  parseRanchUrl,
  type RanchTarget,
  resolveNamedTarget,
} from "./parseRanchUrl"

export function useDeepLinkRouter() {
  const navigate = useNavigate()

  useEffect(() => {
    let unlisten: (() => void) | undefined
    const handler = async (urls: string[]) => {
      for (const url of urls) {
        await dispatch(url)
      }
    }
    const dispatch = async (url: string) => {
      // Id form first; then the named form `ranch://{group}/{environment}/…`
      // — names survive re-creating a connection.
      const idTarget = parseRanchUrl(url)
      const named = idTarget ? null : parseNamedRanchUrl(url)
      if (!idTarget && !named) {
        toast.error(`Invalid ranch:// URL: ${url}`)
        return
      }
      // Verify the connection exists. Cheap (~ms) one-shot list.
      const list = await commands.listConnections()
      if (list.status !== "ok") {
        toast.error(`Could not load connections: ${list.error.message}`)
        return
      }

      let target: RanchTarget
      if (idTarget) {
        target = idTarget
      } else {
        // `named` is non-null here (guarded above).
        const resolution = resolveNamedTarget(named as NonNullable<typeof named>, list.data)
        if (!resolution.ok) {
          toast.error(resolution.reason)
          void navigate({ to: "/" })
          return
        }
        if (resolution.ambiguous) {
          toast.info(
            "Several connections match this link — opened the first. Add ?connection=<name> to pick one.",
          )
        }
        target = resolution.target
      }

      const exists = list.data.some((c) => c.id === target.connectionId)
      if (!exists) {
        toast.error(`We couldn't find connection ${target.connectionId}.`)
        void navigate({ to: "/" })
        return
      }
      await navigateToTarget(target, navigate)
    }
    void onOpenUrl(handler).then((u) => {
      unlisten = u
    })
    return () => {
      try {
        unlisten?.()
      } catch {
        // Best-effort cleanup.
      }
    }
  }, [navigate])
}

async function navigateToTarget(
  target: RanchTarget,
  navigate: ReturnType<typeof useNavigate>,
): Promise<void> {
  if (target.queueName && target.jobId) {
    await navigate({
      to: "/connections/$connectionId/queues/$queueName/$jobId",
      params: {
        connectionId: target.connectionId,
        queueName: target.queueName,
        jobId: target.jobId,
      },
    })
    return
  }
  if (target.queueName) {
    await navigate({
      to: "/connections/$connectionId/queues/$queueName",
      params: { connectionId: target.connectionId, queueName: target.queueName },
    })
    return
  }
  await navigate({
    to: "/connections/$connectionId",
    params: { connectionId: target.connectionId },
  })
}
