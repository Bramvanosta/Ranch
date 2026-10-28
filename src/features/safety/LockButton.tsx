import { LockIcon, LockOpenIcon } from "lucide-react"
import { useEffect, useState } from "react"
import { create } from "zustand"

import { displayChord } from "@/commands/chords"
import { useCommandHandler } from "@/commands/handlers"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useConnections } from "@/data/connections/queries"
import { useLockConnection, useUnlockConnection } from "@/data/safety/mutations"
import { useConnectionLockState } from "@/data/safety/queries"

/** When each connection was unlocked in this session (the unlock lives until Ranch quits). */
const useUnlockedAt = create<{ at: Record<string, number> }>(() => ({ at: {} }))

const minutes = (ms: number) => Math.max(0, Math.floor(ms / 60_000))

/**
 * The read-only lock, quiet but always in sight: a
 * toolbar button on every screen of a production connection.
 *
 *   - locked: `Read-only ⇧⌘U`: a click (or ⇧⌘U) unlocks for this session;
 *   - unlocked: `Unlocked · 13 min`: how long it has been open; a click (or
 *     ⇧⌘L) locks it again now. Quitting Ranch re-locks it.
 *
 * Connection › Unlock for Session / Lock Now are the same actions. The
 * sidebar row carries a lock glyph while locked. Only
 * production-coloured connections start locked: elsewhere there is
 * no lock to show.
 */
export function LockButton({ connectionId }: { connectionId: string }) {
  const lockState = useConnectionLockState(connectionId)
  const unlock = useUnlockConnection()
  const lock = useLockConnection()
  const { data: connections } = useConnections({ staleTime: 30_000 })
  const guarded = connections?.find((c) => c.id === connectionId)?.envColor === "env-production"
  // Loading: show the locked variant, as the connection starts locked.
  const state = lockState.data ?? "locked"
  const unlockedAt = useUnlockedAt((s) => s.at[connectionId])

  useEffect(() => {
    if (state === "unlocked" && unlockedAt === undefined) {
      useUnlockedAt.setState((s) => ({ at: { ...s.at, [connectionId]: Date.now() } }))
    } else if (state === "locked" && unlockedAt !== undefined) {
      useUnlockedAt.setState((s) => {
        const { [connectionId]: _, ...rest } = s.at
        return { at: rest }
      })
    }
  }, [state, unlockedAt, connectionId])

  // A minute is the resolution the label shows.
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    if (state !== "unlocked") return
    const t = setInterval(() => setNow(Date.now()), 15_000)
    return () => clearInterval(t)
  }, [state])

  useCommandHandler("connection.unlock", () => unlock.mutate(connectionId), {
    enabled: lockState.data === "locked" && !unlock.isPending,
  })
  useCommandHandler("connection.lock", () => lock.mutate(connectionId), {
    enabled: lockState.data === "unlocked" && !lock.isPending,
  })

  if (!guarded) return null

  if (state === "locked") {
    return (
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              size="sm"
              variant="default"
              onClick={() => unlock.mutate(connectionId)}
              disabled={unlock.isPending}
              shortcut={displayChord("cmd+shift+u")}
              data-testid="lock-button"
              data-state="locked"
            >
              <LockIcon aria-hidden="true" />
              Read-only
            </Button>
          }
        />
        <TooltipContent>
          Production is read-only: writes are refused. Unlock for this session.
        </TooltipContent>
      </Tooltip>
    )
  }

  const open = minutes(now - (unlockedAt ?? now))
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            size="sm"
            variant="default"
            onClick={() => lock.mutate(connectionId)}
            disabled={lock.isPending}
            data-testid="lock-button"
            data-state="unlocked"
            className="border-(--state-delayed)/50 text-(--state-delayed-text)"
          >
            <LockOpenIcon aria-hidden="true" />
            {`Unlocked · ${open} min`}
          </Button>
        }
      />
      <TooltipContent>
        Writes are allowed until Ranch quits. Click or {displayChord("cmd+shift+l")} to lock now.
      </TooltipContent>
    </Tooltip>
  )
}
