import { type ErrorComponentProps, Link, useRouter } from "@tanstack/react-router"
import { CompassIcon, HomeIcon, RotateCwIcon, TriangleAlertIcon } from "lucide-react"

import { Button } from "@/components/ui/button"

/**
 * UI review 2026-09-19 — what a page shows when it throws, instead of the
 * router's stock red dump ("Something went wrong!" + raw JSON over the whole
 * window). Rendered where the page would have been, so the rail and the
 * sidebar stay usable: the way out is one click away.
 */
export function RouteError({ error, reset }: ErrorComponentProps) {
  const router = useRouter()
  const message = error instanceof Error ? error.message : String(error)

  return (
    <div className="flex h-full items-start justify-center overflow-auto p-8">
      <div
        role="alert"
        data-testid="route-error"
        className="mt-10 flex w-full max-w-xl flex-col items-center gap-4 rounded-xl border border-dashed border-(--line) bg-(--surf)/20 p-8 text-center"
      >
        <span className="flex size-10 items-center justify-center rounded-full bg-(--danger)/10 text-(--state-failed-text)">
          <TriangleAlertIcon className="size-5" aria-hidden="true" />
        </span>
        <div className="flex flex-col gap-1">
          <h1 className="text-base font-semibold">This page ran into a problem</h1>
          <p className="text-sm text-(--text2)">
            The rest of Ranch is fine. Try again, or go back to your connections.
          </p>
        </div>
        <details className="w-full text-left">
          <summary className="cursor-pointer text-xs text-(--text2) hover:text-(--text)">
            Technical details
          </summary>
          <pre className="mt-2 max-h-48 overflow-auto rounded-md border border-(--line)/60 bg-(--surf) p-3 font-mono text-2xs break-words whitespace-pre-wrap text-(--text2)">
            {message}
          </pre>
        </details>
        <div className="flex items-center gap-2">
          <Button
            variant="primary"
            type="button"
            size="sm"
            onClick={() => {
              reset()
              void router.invalidate()
            }}
            className="gap-1.5"
          >
            <RotateCwIcon className="size-3.5" aria-hidden="true" />
            Try again
          </Button>
          <Button
            size="sm"
            variant="default"
            className="gap-1.5"
            render={<Link to="/" />}
            nativeButton={false}
          >
            <HomeIcon className="size-3.5" aria-hidden="true" />
            Home
          </Button>
        </div>
      </div>
    </div>
  )
}

/** A URL that matches no page (a stale deep link, a typo). */
export function RouteNotFound() {
  return (
    <div className="flex h-full items-start justify-center overflow-auto p-8">
      <div
        data-testid="route-not-found"
        className="mt-10 flex w-full max-w-xl flex-col items-center gap-4 rounded-xl border border-dashed border-(--line) bg-(--surf)/20 p-8 text-center"
      >
        <span className="flex size-10 items-center justify-center rounded-full bg-(--surf) text-(--text2)">
          <CompassIcon className="size-5" aria-hidden="true" />
        </span>
        <div className="flex flex-col gap-1">
          <h1 className="text-base font-semibold">Nothing here</h1>
          <p className="text-sm text-(--text2)">This address does not match any page of Ranch.</p>
        </div>
        <Button
          variant="primary"
          size="sm"
          className="gap-1.5"
          render={<Link to="/" />}
          nativeButton={false}
        >
          <HomeIcon className="size-3.5" aria-hidden="true" />
          Home
        </Button>
      </div>
    </div>
  )
}
