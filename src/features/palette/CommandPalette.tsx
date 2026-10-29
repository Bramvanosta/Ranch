/**
 * cmd-K command palette.
 *
 * Overlay (max-width 600px) with a single Input + grouped result list.
 * Fuzzy match via the in-repo `fuzzyScore`. Recent items persist via
 * `kv_settings` (the write-through happens here on `pushRecent`, not in
 * the store, to keep the store IO-free).
 *
 * Anchored near the top of the window instead of
 * centred (a centred list jumps every time the results change height);
 * queues are searchable (from the lists Ranch already holds, no Redis
 * call); each row says what it is with an icon; the keys are written down.
 */

import { useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { SearchIcon } from "lucide-react"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useCapabilityLookup } from "@/capabilities"
import { displayChord } from "@/commands/chords"
import { commandState, runCommand, useCommandHandlers } from "@/commands/handlers"
import { COMMANDS } from "@/commands/registry"
import { Kbd } from "@/components/data/Kbd"
import { useConnections } from "@/data/connections/queries"
import { useGroups } from "@/data/groups/queries"
import { useRetryJob } from "@/data/jobs/mutations"
import { useJobIdSearch } from "@/data/jobs/queries"
import { queueKeys } from "@/data/queues/keys"
import { paletteRecents } from "@/data/settings/settings"
import { readSettingOrDefault, writeSetting } from "@/data/settings/useSetting"
import { hostPort } from "@/domain/bullmqEndpoint"
import { stateLabel } from "@/domain/jobStates"
import { countOf } from "@/domain/stateCounts"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import { useKeyboardShortcut } from "@/hooks/useKeyboardShortcut"
import { useSelectedConnectionId } from "@/hooks/useSelectedConnectionId"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import type { Connection, Queue } from "@/lib/tauri/bindings"
import { openRouteOrSay } from "@/lib/windows/openRoute"
import { useOpenSettings } from "@/lib/windows/settingsWindow"
import { type RecentItem, useCommandPaletteStore } from "@/stores/useCommandPaletteStore"
import { useSidebarUiStore } from "@/stores/useSidebarUiStore"
import { PaletteDialog } from "./PaletteDialog"
import { PaletteResults } from "./PaletteResults"
import { AGENTS_PAGE, type PaletteItem, paletteSections, SETTINGS_ACTIONS } from "./paletteItems"

export function CommandPalette() {
  const aiOn = useAiEnabled()
  const open = useCommandPaletteStore((s) => s.open)
  const setOpen = useCommandPaletteStore((s) => s.setOpen)
  const recents = useCommandPaletteStore((s) => s.recents)
  const pushRecent = useCommandPaletteStore((s) => s.pushRecent)
  const hydrateRecents = useCommandPaletteStore((s) => s.hydrateRecents)
  const openSettings = useOpenSettings()
  const [query, setQuery] = useState("")
  const [highlight, setHighlight] = useState(0)
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  // The connection on screen, if any: its queues are offered before any typing.
  const currentConnectionId = useSelectedConnectionId()
  const inputRef = useRef<HTMLInputElement | null>(null)
  const resultsRef = useRef<HTMLDivElement | null>(null)

  // Hydrate recents on mount.
  useEffect(() => {
    let cancelled = false
    void (async () => {
      const stored = await readSettingOrDefault(paletteRecents)
      if (!cancelled) hydrateRecents(stored)
    })()
    return () => {
      cancelled = true
    }
  }, [hydrateRecents])

  // Global cmd-K binding.
  useKeyboardShortcut("cmd+k", (e) => {
    e.preventDefault()
    setOpen(true)
  })

  // Reset query + highlight on each open (with the text handed over, if any).
  const initialQuery = useCommandPaletteStore((s) => s.initialQuery)
  useEffect(() => {
    if (open) {
      setQuery(initialQuery)
      setHighlight(0)
      // Defer until the dialog mounts.
      setTimeout(() => inputRef.current?.focus(), 0)
    }
  }, [open, initialQuery])

  // Connections: the key the sidebar and the home page already keep warm, so
  // the list is there on the first keystroke.
  const connectionsQuery = useConnections({ enabled: open, staleTime: 30_000 })

  // Groups are searchable; selecting one reveals it in the sidebar.
  const groupsQuery = useGroups({ enabled: open, staleTime: 30_000 })

  const copy = useCopyToClipboard()
  // A path with its search (`/connections/c1?view=failed`): routed as an href.
  const go = useCallback(
    (to: string) => {
      setOpen(false)
      void navigate({ href: to } as never)
    },
    [navigate, setOpen],
  )
  // ⇥ on a result lists what can be done with it.
  const [subject, setSubject] = useState<PaletteItem | null>(null)
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new search or a new opening leaves the actions.
  useEffect(() => setSubject(null), [query, open])

  const handlers = useCommandHandlers((s) => s.handlers)
  const allItems: PaletteItem[] = useMemo(() => {
    // Only a Redis endpoint has a host and port to copy.
    const copyHostPort = (c: Connection) => {
      const at = hostPort(c)
      return at
        ? [{ id: "host", label: "Copy host and port", run: () => void copy(at, "host and port") }]
        : []
    }
    const conns: PaletteItem[] = (connectionsQuery.data ?? []).map((c) => ({
      kind: "connection",
      id: c.id,
      label: c.name,
      // Group / environment first: "acme prod" finds the right Redis.
      hint: `${c.groupName} / ${c.envLabel} · ${hostPort(c) ?? c.backend}`,
      to: `/connections/${c.id}`,
      actions: [
        { id: "open", label: "Open", run: () => go(`/connections/${c.id}`) },
        {
          id: "tab",
          label: "Open in New Tab",
          run: () => void openRouteOrSay(`/connections/${c.id}`, { tab: true }),
        },
        {
          id: "failed",
          label: "Open its failed jobs",
          run: () => go(`/connections/${c.id}?view=failed`),
        },
        ...copyHostPort(c),
      ],
    }))
    const sidebar = useSidebarUiStore.getState()
    const groups: PaletteItem[] = (groupsQuery.data ?? []).flatMap((g) => [
      {
        kind: "group" as const,
        id: `go-group-${g.id}`,
        label: `Go to group: ${g.name}`,
        hint: `${g.connectionCount} connection${g.connectionCount === 1 ? "" : "s"}`,
        run: () => sidebar.revealGroup(g.id),
      },
      {
        kind: "group" as const,
        id: `rename-group-${g.id}`,
        label: `Rename group: ${g.name}`,
        run: () => sidebar.revealGroup(g.id, { rename: true }),
      },
    ])
    // Every registry command that applies now, with the same
    // chord as the menu bar. Help › Keyboard Shortcuts lists them all.
    const actions: PaletteItem[] = COMMANDS.flatMap((c) => {
      const state = commandState(handlers, c.id)
      if (!state.enabled) return []
      return [
        {
          kind: "action" as const,
          id: `command-${c.id}`,
          label: state.label.replace(/…$/, ""),
          hint: c.shortcut ? displayChord(c.shortcut) : c.menu,
          run: () => runCommand(c.id),
        },
      ]
    })
    // Queues of every connection Ranch has already listed (the open one, at
    // least): read from the query cache when the palette opens, no Redis call.
    const queues: PaletteItem[] = open
      ? queryClient.getQueriesData<Queue[]>({ queryKey: queueKeys.all }).flatMap(([key, list]) => {
          const connectionId = String(key[1] ?? "")
          const conn = (connectionsQuery.data ?? []).find((c) => c.id === connectionId)
          if (!conn || !Array.isArray(list)) return []
          return list.map((q) => {
            const route = `/connections/${connectionId}/queues/${encodeURIComponent(q.id)}`
            const failed = countOf(q.counts, "failed")
            return {
              kind: "queue" as const,
              id: `${connectionId}:${q.id}`,
              label: q.name,
              // Where it is and how it is doing.
              hint: `${failed > 0 ? `${failed.toLocaleString()} failed · ` : ""}${conn.name} · ${conn.envLabel}`,
              to: route,
              actions: [
                { id: "open", label: "Open", run: () => go(route) },
                {
                  id: "tab",
                  label: "Open in New Tab",
                  run: () => void openRouteOrSay(route, { tab: true }),
                },
                {
                  id: "failed",
                  label: "Open its failed jobs by cause",
                  run: () => go(`${route}?view=jobs&state=failed&group=cause`),
                },
                {
                  id: "key",
                  label: "Copy queue ID",
                  run: () => void copy(q.id, "queue ID"),
                },
              ],
            }
          })
        })
      : []
    // The Agents page, while AI features are on.
    const pages = aiOn ? [...SETTINGS_ACTIONS, AGENTS_PAGE] : SETTINGS_ACTIONS
    return [...conns, ...queues, ...groups, ...actions, ...pages]
  }, [connectionsQuery.data, groupsQuery.data, open, queryClient, handlers, go, copy, aiOn])

  // `#1211`: that job in the connection on screen (every queue
  // Ranch has listed for it), or across the listed connections otherwise.
  const jobId = /^#(\S+)$/.exec(query.trim())?.[1] ?? null
  const jobSearch = useJobIdSearch(currentConnectionId ?? null, jobId, open)
  // A job's actions are those its connection's backend declares.
  const capabilitiesOf = useCapabilityLookup()
  // Toasts the outcome and refreshes the job and its lists.
  const retry = useRetryJob()
  const jobItems: PaletteItem[] = useMemo(
    () =>
      (jobSearch.data ?? []).map(({ connectionId, queueId, job }) => {
        const route = `/connections/${connectionId}/queues/${encodeURIComponent(queueId)}/${encodeURIComponent(job.jobRef.id)}`
        const conn = (connectionsQuery.data ?? []).find((c) => c.id === connectionId)
        return {
          kind: "job" as const,
          id: `job:${connectionId}:${queueId}:${job.jobRef.id}`,
          label: `${job.name || "(unnamed)"} #${job.jobRef.id}`,
          hint: `${stateLabel(job.state, capabilitiesOf(connectionId).jobStates)} · ${job.jobRef.queueName}${conn ? ` · ${conn.name} · ${conn.envLabel}` : ""}`,
          to: route,
          actions: [
            { id: "open", label: "Open", run: () => go(route) },
            {
              id: "window",
              label: "Open in New Window",
              run: () => void openRouteOrSay(route, { tab: false }),
            },
            // Only a failed job is retried.
            ...(job.state === "failed" && capabilitiesOf(connectionId).can("jobActions.single")
              ? [
                  {
                    id: "retry",
                    label: "Retry",
                    run: () => retry.mutate({ connectionId, queueId, jobId: job.jobRef.id }),
                  },
                ]
              : []),
            { id: "copy", label: "Copy job ID", run: () => void copy(job.jobRef.id, "job ID") },
          ],
        }
      }),
    [jobSearch.data, connectionsQuery.data, go, copy, capabilitiesOf, retry.mutate],
  )

  const grouped = useMemo(
    () => paletteSections({ allItems, recents, query, currentConnectionId, jobId, jobItems }),
    [allItems, recents, query, currentConnectionId, jobId, jobItems],
  )

  // Flatten for keyboard navigation.
  const flatItems: PaletteItem[] = useMemo(() => Object.values(grouped).flat(), [grouped])
  const subjectActions = subject?.actions ?? []

  const persistRecents = useCallback((items: RecentItem[]) => {
    writeSetting(paletteRecents, items).catch(() => {
      // Best effort: the recents of this session stay in memory.
    })
  }, [])

  const select = useCallback(
    (item: PaletteItem) => {
      // Sidebar intents run in place; they are not navigations
      // and do not belong in recents.
      if (item.run) {
        setOpen(false)
        item.run()
        return
      }
      if (!item.to) return
      // Push to recents (non-recent kinds only).
      if (item.kind !== "recent") {
        pushRecent({
          kind:
            item.kind === "settings" ? "settings" : item.kind === "queue" ? "queue" : "connection",
          id: item.id,
          label: item.label,
          hint: item.hint,
          to: item.to,
        })
        // The store mutation is synchronous; re-read after.
        const next = useCommandPaletteStore.getState().recents
        persistRecents(next)
      }
      setOpen(false)
      // Settings is a window of its own on the desktop.
      if (item.to === "/settings") {
        openSettings()
        return
      }
      // The router's typed navigate doesn't accept arbitrary strings,
      // but we can route via `to: item.to as any` after asserting at
      // runtime that the path is one we created.
      void navigate({ to: item.to } as never)
    },
    [navigate, openSettings, persistRecents, pushRecent, setOpen],
  )

  // Keyboard navigation inside the palette.
  // `j` / `k` only move the highlight together with Ctrl: this handler sits on
  // the search input, where a bare `j` or `k` must type the letter ("jobs").
  const onKeyDown = (e: React.KeyboardEvent) => {
    const down = e.key === "ArrowDown" || (e.ctrlKey && (e.key === "j" || e.key === "n"))
    const up = e.key === "ArrowUp" || (e.ctrlKey && (e.key === "k" || e.key === "p"))
    const count = subject ? subjectActions.length : flatItems.length
    if (e.key === "Tab" && !e.shiftKey && !subject) {
      const item = flatItems[highlight]
      if (item?.actions?.length) {
        e.preventDefault()
        setSubject(item)
        setHighlight(0)
      }
    } else if (subject && (e.key === "Tab" || (e.key === "Backspace" && query === ""))) {
      // ⇧⇥ (or ⌫ on an empty search) goes back to the results.
      e.preventDefault()
      setSubject(null)
      setHighlight(0)
    } else if (down) {
      e.preventDefault()
      setHighlight((h) => Math.min(count - 1, h + 1))
    } else if (up) {
      e.preventDefault()
      setHighlight((h) => Math.max(0, h - 1))
    } else if (e.key === "Enter") {
      e.preventDefault()
      if (subject) {
        const action = subjectActions[highlight]
        if (action) {
          setOpen(false)
          action.run()
        }
        return
      }
      const item = flatItems[highlight]
      if (item) select(item)
    }
  }

  // Keep the highlighted row in view while arrowing through a long list.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `highlight` is the trigger
  useEffect(() => {
    resultsRef.current
      ?.querySelector<HTMLElement>('[data-active="true"]')
      ?.scrollIntoView?.({ block: "nearest" })
  }, [highlight])

  return (
    <PaletteDialog open={open} onOpenChange={setOpen} initialFocus={inputRef}>
      <div className="flex items-center gap-2 border-b px-4">
        <SearchIcon className="size-4 shrink-0 text-(--text2)" aria-hidden="true" />
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
            setHighlight(0)
          }}
          onKeyDown={onKeyDown}
          placeholder="Queues, connections, actions, or #job"
          aria-label="Command palette search"
          className="min-w-0 flex-1 bg-transparent py-3 text-base outline-none focus:ring-0"
          data-testid="command-palette-input"
        />
        <Kbd>esc</Kbd>
      </div>
      <div
        ref={resultsRef}
        className="max-h-[min(400px,60vh)] overflow-y-auto py-2"
        data-testid="command-palette-results"
      >
        <PaletteResults
          subject={subject}
          subjectActions={subjectActions}
          grouped={grouped}
          flatItems={flatItems}
          highlight={highlight}
          onHighlight={setHighlight}
          onSelect={select}
          onRunAction={(action) => {
            setOpen(false)
            action.run()
          }}
          emptyMessage={
            jobId !== null
              ? jobSearch.isFetching
                ? `Looking for job #${jobId}`
                : `No job #${jobId} in the queues Ranch has listed.`
              : `Nothing matches “${query}”.`
          }
        />
      </div>
      <div className="flex items-center gap-4 border-t bg-(--surf)/20 px-4 py-2 text-2xs text-(--text2)">
        <span>
          <Kbd label="Up and down arrows">↑↓</Kbd> navigate
        </span>
        <span>
          <Kbd label="Return">↵</Kbd> open
        </span>
        <span>
          <Kbd label="Tab">⇥</Kbd> actions
        </span>
        <span className="ml-auto">
          <Kbd label={displayChord("cmd+k")}>{displayChord("cmd+k")}</Kbd> anywhere
        </span>
      </div>
    </PaletteDialog>
  )
}
