import { useIsMutating, useMutation } from "@tanstack/react-query"
import { listen } from "@tauri-apps/api/event"
import { useEffect, useState } from "react"
import { unwrap } from "@/data/result"
import { commands, type UpdateProgressEvent } from "@/lib/tauri/bindings"
import { updaterKeys } from "./keys"

/** The event `install_update` reports each phase on. */
export const UPDATE_PROGRESS_EVENT = "updater:progress"

export type InstallPhase = UpdateProgressEvent["phase"]

/**
 * How far the install has got: its phase, and the download's bytes, kept
 * once the download is over so the finished step can still say its size.
 */
export interface InstallProgress {
  phase: InstallPhase
  received: number | null
  total: number | null
}

function advance(prev: InstallProgress | null, event: UpdateProgressEvent): InstallProgress {
  if (event.phase === "downloading") {
    return { phase: event.phase, received: event.received, total: event.total }
  }
  return { phase: event.phase, received: prev?.received ?? null, total: prev?.total ?? null }
}

/**
 * Download, verify and install the published update, then restart. On
 * success the app restarts and the mutation never resolves. A failure is
 * `error`, for the sheet to show in place.
 *
 * `progress` is the last phase reported (`null` before the first one); it
 * stays on the phase that failed. `running` is true while any install runs,
 * this one or one started from the other place that offers it (the sidebar
 * notice, Settings › Updates): the backend refuses a second one.
 */
export function useInstallUpdate() {
  const [progress, setProgress] = useState<InstallProgress | null>(null)

  useEffect(() => {
    let unlisten: (() => void) | undefined
    let cancelled = false
    void listen<UpdateProgressEvent>(UPDATE_PROGRESS_EVENT, (event) => {
      setProgress((prev) => advance(prev, event.payload))
    })
      .then((fn) => {
        if (cancelled) fn()
        else unlisten = fn
      })
      .catch(() => {
        // No event bus (tests): the install runs, its phases go unseen.
      })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  const mutation = useMutation({
    mutationKey: updaterKeys.install,
    mutationFn: async () => {
      // A retry starts over: the phase a failed try reached is no longer news.
      setProgress(null)
      unwrap(await commands.installUpdate())
    },
  })
  const running = useIsMutating({ mutationKey: updaterKeys.install }) > 0

  return {
    install: () => mutation.mutate(),
    /** Forget a failed try (the sheet closed): the next one starts from the top. */
    reset: () => {
      mutation.reset()
      setProgress(null)
    },
    isPending: mutation.isPending,
    error: mutation.isError ? mutation.error : null,
    progress,
    running,
  }
}
