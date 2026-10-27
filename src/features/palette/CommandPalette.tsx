/**
 * cmd-K command palette.
 *
 * Overlay (max-width 600px) with a single Input + grouped result list.
 * Fuzzy match via the in-repo `fuzzyScore`. Recent items persist via
 * `kv_settings` (the write-through happens here on `pushRecent`, not in
 * the store, to keep the store IO-free).
 *
 * UI review 2026-09-19 — anchored near the top of the window instead of
 * centred (a centred list jumps every time the results change height);
 * queues are searchable (from the lists Ranch already holds — no Redis
 * call); each row says what it is with an icon; the keys are written down.
 */

import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate, useRouterState } from "@tanstack/react-router"
import {
  ArrowRightIcon,
  BriefcaseIcon,
  DatabaseIcon,
  FolderIcon,
  HistoryIcon,
  LayersIcon,
  type LucideIcon,
  SearchIcon,
  ZapIcon,
} from "lucide-react"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import { displayChord } from "@/commands/chords"
import { commandState, runCommand, useCommandHandlers } from "@/commands/handlers"
import { COMMANDS } from "@/commands/registry"
import { Kbd } from "@/components/data/Kbd"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import { useKeyboardShortcut } from "@/hooks/useKeyboardShortcut"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { commands, type Queue } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { openRouteOrSay } from "@/lib/windows/openRoute"
import { useOpenSettings } from "@/lib/windows/settingsWindow"
import { type RecentItem, useCommandPaletteStore } from "@/stores/useCommandPaletteStore"
import { useSidebarUiStore } from "@/stores/useSidebarUiStore"

import { rankBy } from "./fuzzyMatch"
import { PaletteDialog } from "./PaletteDialog"

const PALETTE_RECENTS_KEY = "palette.recents"

interface PaletteItem {
  kind: "connection" | "queue" | "settings" | "recent" | "group" | "action" | "job"
  id: string
  label: string
  hint?: string
  /** Route to navigate to. Absent for sidebar intents, which use `run`. */
  to?: string
  /** A sidebar intent (new group, reveal / rename a group…). Not added to recents. */
  run?: () => void
  /** What ⇥ offers for this result. */
  actions?: PaletteAction[]
}

/** One of the actions ⇥ lists for a result. */
interface PaletteAction {
  id: string
  label: string
  run: () => void
}

const KIND_ICONS: Record<PaletteItem["kind"], LucideIcon> = {
  job: BriefcaseIcon,
  recent: HistoryIcon,
  connection: DatabaseIcon,
  queue: LayersIcon,
  group: FolderIcon,
  action: ZapIcon,
  settings: ArrowRightIcon,
}

const SETTINGS_ACTIONS: PaletteItem[] = [
  // The home page: every connection, grouped, with live health.
  { kind: "settings", id: "go-home", label: "Go to Home — all connections", to: "/" },
  { kind: "settings", id: "go-settings", label: "Go to Settings", to: "/settings" },
  { kind: "settings", id: "go-audit", label: "Go to Audit log", to: "/audit" },
]

const AGENTS_PAGE: PaletteItem = {
  kind: "settings",
  id: "go-agents",
  label: "Go to Agents",
  to: "/agents",
}

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
  const currentConnectionId = useRouterState({
    select: (s) => /^\/connections\/([^/]+)/.exec(s.location.pathname)?.[1] ?? null,
  })
  const inputRef = useRef<HTMLInputElement | null>(null)
  const resultsRef = useRef<HTMLDivElement | null>(null)

  // Hydrate recents on mount.
  useEffect(() => {
    let cancelled = false
    void (async () => {
      const r = await commands.getKvSetting(PALETTE_RECENTS_KEY)
      if (cancelled) return
      if (r.status !== "ok" || r.data === null) return
      try {
        const parsed: RecentItem[] = JSON.parse(r.data)
        if (Array.isArray(parsed)) hydrateRecents(parsed)
      } catch {
        // Corrupt blob — defensively ignore.
      }
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
  const connectionsQuery = useQuery({
    queryKey: ["connections"],
    queryFn: async () => {
      const r = await commands.listConnections()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    enabled: open,
    staleTime: 30_000,
  })

  // Groups are searchable; selecting one reveals it in the sidebar.
  const groupsQuery = useQuery({
    queryKey: ["palette.groups"],
    queryFn: async () => {
      const r = await commands.listGroups()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    enabled: open,
    staleTime: 30_000,
  })

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
    const conns: PaletteItem[] = (connectionsQuery.data ?? []).map((c) => ({
      kind: "connection",
      id: c.id,
      label: c.name,
      // Group / environment first: "acme prod" finds the right Redis.
      hint: `${c.groupName} / ${c.envLabel} · ${c.host}:${c.port}`,
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
        {
          id: "host",
          label: "Copy host and port",
          run: () => void copy(`${c.host}:${c.port}`, "host and port"),
        },
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
    // least): read from the query cache when the palette opens — no Redis call.
    const queues: PaletteItem[] = open
      ? queryClient.getQueriesData<Queue[]>({ queryKey: ["queues"] }).flatMap(([key, list]) => {
          const connectionId = String(key[1] ?? "")
          const conn = (connectionsQuery.data ?? []).find((c) => c.id === connectionId)
          if (!conn || !Array.isArray(list)) return []
          return list.map((q) => {
            const route = `/connections/${connectionId}/queues/${encodeURIComponent(q.queueRef.name)}`
            const failed = Number(q.counters?.failed ?? 0)
            return {
              kind: "queue" as const,
              id: `${connectionId}:${q.queueRef.prefix}:${q.queueRef.name}`,
              label: q.queueRef.name,
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
                  label: "Copy queue key",
                  run: () => void copy(`${q.queueRef.prefix}:${q.queueRef.name}`, "queue key"),
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
  const jobSearch = useQuery({
    queryKey: ["palette.job", currentConnectionId, jobId],
    queryFn: async () => {
      const targets = queryClient
        .getQueriesData<Queue[]>({ queryKey: ["queues"] })
        .flatMap(([key, list]) => {
          const connectionId = String(key[1] ?? "")
          if (currentConnectionId && connectionId !== currentConnectionId) return []
          return Array.isArray(list)
            ? list.map((q) => ({ connectionId, queueRef: q.queueRef }))
            : []
        })
        .slice(0, 40)
      const found = await Promise.all(
        targets.map(async (t) => {
          const r = await commands.getJob({ ...t, jobId: jobId ?? "" })
          return r.status === "ok" ? { ...t, job: r.data } : null
        }),
      )
      return found.filter((f) => f !== null)
    },
    enabled: open && jobId !== null,
    staleTime: 5_000,
  })
  const jobItems: PaletteItem[] = useMemo(
    () =>
      (jobSearch.data ?? []).map(({ connectionId, queueRef, job }) => {
        const route = `/connections/${connectionId}/queues/${encodeURIComponent(queueRef.name)}/${encodeURIComponent(job.jobRef.id)}`
        const conn = (connectionsQuery.data ?? []).find((c) => c.id === connectionId)
        return {
          kind: "job" as const,
          id: `job:${connectionId}:${queueRef.name}:${job.jobRef.id}`,
          label: `${job.name || "(unnamed)"} #${job.jobRef.id}`,
          hint: `${job.state} · ${queueRef.name}${conn ? ` · ${conn.name}` : ""}`,
          to: route,
          actions: [
            { id: "open", label: "Open", run: () => go(route) },
            {
              id: "window",
              label: "Open in New Window",
              run: () => void openRouteOrSay(route, { tab: false }),
            },
            {
              id: "retry",
              label: "Retry",
              run: () => void commands.retryJob({ connectionId, queueRef, jobId: job.jobRef.id }),
            },
            { id: "copy", label: "Copy job ID", run: () => void copy(job.jobRef.id, "job ID") },
          ],
        }
      }),
    [jobSearch.data, connectionsQuery.data, go, copy],
  )

  const grouped = useMemo(() => {
    const recentItems: PaletteItem[] = recents.map((r) => ({
      kind: "recent",
      id: `${r.kind}-${r.id}`,
      label: r.label,
      hint: r.hint,
      to: r.to,
    }))
    // Three sections: Go to (queues with their failures,
    // connections, groups, pages), Actions on the current target (the
    // registry, with the menu's chords) and Jobs (`#id`).
    if (query === "") {
      const here = currentConnectionId
        ? allItems
            .filter((i) => i.kind === "queue" && i.id.startsWith(`${currentConnectionId}:`))
            .slice(0, 8)
        : []
      return {
        Recent: recentItems,
        "Go to": [
          ...here,
          ...allItems.filter((i) => i.kind === "connection"),
          ...allItems.filter((i) => i.kind === "group" && i.id.startsWith("go-group-")),
          ...allItems.filter((i) => i.kind === "settings"),
        ],
        Actions: allItems.filter((i) => i.kind === "action"),
        Jobs: [],
      }
    }
    if (jobId !== null) {
      return { Recent: [], "Go to": [], Actions: [], Jobs: jobItems }
    }
    // Connections also match on their "group / environment" hint.
    const matchAll = rankBy(allItems, query, (i) =>
      i.kind === "connection" || i.kind === "queue" ? `${i.label} ${i.hint ?? ""}` : i.label,
    )
    return {
      Recent: rankBy(recentItems, query, (i) => i.label),
      "Go to": [
        ...matchAll.filter((i) => i.kind === "queue").slice(0, 8),
        ...matchAll.filter(
          (i) => i.kind === "connection" || i.kind === "group" || i.kind === "settings",
        ),
      ],
      Actions: matchAll.filter((i) => i.kind === "action"),
      Jobs: [],
    }
  }, [allItems, recents, query, currentConnectionId, jobId, jobItems])

  // Flatten for keyboard navigation.
  const flatItems: PaletteItem[] = useMemo(() => Object.values(grouped).flat(), [grouped])
  const subjectActions = subject?.actions ?? []

  const persistRecents = useCallback((items: RecentItem[]) => {
    void commands.setKvSetting(PALETTE_RECENTS_KEY, JSON.stringify(items))
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
  // the search input, and bare `j` / `k` used to be swallowed — nobody could
  // type "jobs" or any name containing those letters.
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
        {subject ? (
          <div className="px-2 pb-2" data-testid="command-palette-actions">
            <div className="px-2 pb-1 pt-2 text-2xs font-semibold tracking-wide text-(--text2) uppercase">
              Actions for {subject.label}
            </div>
            {subjectActions.map((action, idx) => (
              <button
                key={action.id}
                type="button"
                onClick={() => {
                  setOpen(false)
                  action.run()
                }}
                onMouseEnter={() => setHighlight(idx)}
                data-active={idx === highlight ? "true" : undefined}
                className={cn(
                  "flex w-full items-center gap-2.5 rounded-control px-2 py-1.5 text-left text-sm",
                  idx === highlight ? "bg-(--surf2)" : "hover:bg-(--surf2)/60",
                )}
                data-testid={`command-palette-action-${action.id}`}
              >
                {action.label}
              </button>
            ))}
          </div>
        ) : null}
        {subject
          ? null
          : Object.entries(grouped).map(([group, items]) =>
              items.length === 0 ? null : (
                <div key={group} className="px-2 pb-2">
                  <div className="px-2 pb-1 pt-2 text-2xs font-semibold uppercase tracking-wide text-(--text2)">
                    {group}
                  </div>
                  {items.map((item) => {
                    const idx = flatItems.indexOf(item)
                    const active = idx === highlight
                    const Icon = KIND_ICONS[item.kind]
                    return (
                      <button
                        key={`${group}-${item.id}`}
                        type="button"
                        onClick={() => select(item)}
                        onMouseEnter={() => setHighlight(idx)}
                        data-active={active ? "true" : undefined}
                        className={cn(
                          "flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm",
                          active ? "bg-(--surf2) text-(--text)" : "hover:bg-(--surf)/50",
                        )}
                        data-testid={`command-palette-item-${item.id}`}
                      >
                        <Icon className="size-3.5 shrink-0 text-(--text2)" aria-hidden="true" />
                        <span className="min-w-0 break-words">{item.label}</span>
                        {item.hint ? (
                          <span className="ml-auto min-w-0 pl-4 text-right text-xs break-words text-(--text2)">
                            {item.hint}
                          </span>
                        ) : null}
                        {item.actions?.length && active ? (
                          <Kbd label="Tab for actions" className="shrink-0">
                            ⇥
                          </Kbd>
                        ) : null}
                      </button>
                    )
                  })}
                </div>
              ),
            )}
        {!subject && flatItems.length === 0 ? (
          <p className="px-4 py-6 text-center text-sm text-(--text2)">
            {jobId !== null
              ? jobSearch.isFetching
                ? `Looking for job #${jobId}`
                : `No job #${jobId} in the queues Ranch has listed.`
              : `Nothing matches “${query}”.`}
          </p>
        ) : null}
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
          <Kbd label="Command K">⌘K</Kbd> anywhere
        </span>
      </div>
    </PaletteDialog>
  )
}
