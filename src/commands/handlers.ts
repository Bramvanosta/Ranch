import { useEffect, useRef } from "react"
import { create } from "zustand"

import { commandById } from "./registry"

/**
 * What can be done right now. Components and routes register a
 * handler for each registry command they own, with whether it applies and,
 * optionally, a label for this context ("Resume “emails”", "Retry 3 Jobs").
 * This store is the command context: the open connection,
 * queue, selection and lock state show up as which handlers are enabled.
 *
 * Several owners may handle one command (a selection and the focused row
 * both answer "Retry"): the highest priority wins, then the latest.
 */
export interface CommandHandler {
  token: symbol
  run: { current: () => void }
  enabled: boolean
  label?: string
  priority: number
  order: number
}

interface HandlersState {
  handlers: Record<string, CommandHandler[]>
}

let order = 0

export const useCommandHandlers = create<HandlersState>(() => ({ handlers: {} }))

/** The handler that answers `id` now, if any (enabled or not). */
export function activeHandler(
  handlers: Record<string, CommandHandler[]>,
  id: string,
): CommandHandler | undefined {
  const list = handlers[id] ?? []
  const enabled = list.filter((h) => h.enabled)
  const pool = enabled.length > 0 ? enabled : list
  return [...pool].sort((a, b) => b.priority - a.priority || b.order - a.order)[0]
}

export interface CommandState {
  enabled: boolean
  label: string
}

export function commandState(handlers: Record<string, CommandHandler[]>, id: string): CommandState {
  const h = activeHandler(handlers, id)
  return {
    enabled: Boolean(h?.enabled),
    label: h?.label ?? commandById.get(id)?.label ?? id,
  }
}

/** Where a run comes from: the native menu, a key chord, or a click in the app. */
export type CommandSource = "menu" | "key" | "ui"

let lastRun: { id: string; source: CommandSource; at: number } = { id: "", source: "ui", at: 0 }

/**
 * Run a registry command if something can. Returns whether it ran.
 * One key press can reach both the native menu (its accelerator) and the
 * window's key listener: a menu run and a key run of the same command within
 * 150 ms are the same press, and the second is dropped.
 */
export function runCommand(id: string, source: CommandSource = "ui"): boolean {
  const h = activeHandler(useCommandHandlers.getState().handlers, id)
  if (!h?.enabled) return false
  const now = Date.now()
  const echo =
    lastRun.id === id &&
    now - lastRun.at < 150 &&
    lastRun.source !== source &&
    source !== "ui" &&
    lastRun.source !== "ui"
  lastRun = { id, source, at: now }
  if (echo) return true
  h.run.current()
  return true
}

/** Register what this component does for registry command `id` while it is mounted. */
export function useCommandHandler(
  id: string,
  run: () => void,
  {
    enabled = true,
    label,
    priority = 0,
  }: { enabled?: boolean; label?: string; priority?: number } = {},
) {
  const runRef = useRef(run)
  runRef.current = run
  const tokenRef = useRef<symbol>(Symbol(id))

  useEffect(() => {
    const token = tokenRef.current
    const entry: CommandHandler = { token, run: runRef, enabled, label, priority, order: ++order }
    useCommandHandlers.setState((s) => ({
      handlers: {
        ...s.handlers,
        [id]: [...(s.handlers[id] ?? []).filter((h) => h.token !== token), entry],
      },
    }))
  }, [id, enabled, label, priority])

  useEffect(() => {
    const token = tokenRef.current
    return () =>
      useCommandHandlers.setState((s) => ({
        handlers: { ...s.handlers, [id]: (s.handlers[id] ?? []).filter((h) => h.token !== token) },
      }))
  }, [id])
}
