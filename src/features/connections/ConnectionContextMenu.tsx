import { useDraggable, useDroppable } from "@dnd-kit/core"
import { useQueryClient } from "@tanstack/react-query"
import { Link, useParams } from "@tanstack/react-router"
import { ChevronRightIcon, LockIcon } from "lucide-react"

import { EnvTag } from "@/components/data/Env"
import { separator, TargetMenu } from "@/components/menus/TargetMenu"
import { useQueues } from "@/features/queues/useQueues"
import { useConnectionLockState } from "@/features/safety/useConnectionLockState"
import type { Connection } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { openRouteOrSay } from "@/lib/windows/openRoute"

import { failedTotal } from "./queueTriage"
import { ReachabilityIndicator } from "./ReachabilityIndicator"
import { SidebarQueues } from "./SidebarQueues"
import type { SidebarDndData } from "./sidebarDrop"
import { useConnectionMenu } from "./useConnectionMenu"
import { useSetConnectionCollapsed } from "./useSidebarQueues"

interface ConnectionContextMenuProps {
  connection: Connection
}

/**
 * Connection row + right-click context menu, in one component.
 *
 * The row content (env stripe + icon + name + reachability) is folded into
 * this component because base-ui's `ContextMenuTrigger` uses a `render` prop
 * (instead of Radix's `asChild`) — render takes a React element to use as the
 * trigger, and the trigger BECOMES that element. So the trigger renders the
 * `<li>` directly with all the row content inside.
 *
 * The environment is a tag after the name (the tree has no environment
 * level). Items: Edit, Duplicate, Environment (retag), Move to Group (keeps
 * the tag), Environment Color (recolours the tag), Remove.
 */
export function ConnectionContextMenu({ connection }: ConnectionContextMenuProps) {
  // The actions live in `useConnectionMenu`, shared with the
  // home page cards.
  const menu = useConnectionMenu(connection)

  // The row is draggable onto another group and is itself a drop target
  // (dropping on a connection = dropping on its group). Pointer-driven; the
  // keyboard path is the Move to Group menu.
  const dndData: SidebarDndData = {
    type: "connection",
    connectionId: connection.id,
    groupId: connection.groupId,
  }
  const dndId = `connection:${connection.id}`
  const draggable = useDraggable({ id: dndId, data: dndData })
  const droppable = useDroppable({ id: dndId, data: dndData })

  const params = useParams({ strict: false }) as { connectionId?: string; queueName?: string }
  const isSelected = params.connectionId === connection.id && !params.queueName

  // Failure count, lock glyph, and the queues when unfolded.
  // A background reader: alone, it polls every 10 s; the open
  // connection's screen makes it every second.
  const queues = useQueues(connection.id, { background: true })
  const failures = failedTotal(queues.data)
  const { data: lockState } = useConnectionLockState(connection.id)
  const setCollapsed = useSetConnectionCollapsed()
  const open = !connection.sidebarCollapsed
  const toggle = () => setCollapsed.mutate({ connectionId: connection.id, collapsed: open })
  // Fold every connection of the sidebar at once.
  const queryClient = useQueryClient()
  const collapseAll = () => {
    for (const c of queryClient.getQueryData<Connection[]>(["connections"]) ?? []) {
      if (!c.sidebarCollapsed) setCollapsed.mutate({ connectionId: c.id, collapsed: true })
    }
  }

  return (
    <li data-sidebar-item className={cn(draggable.isDragging && "opacity-50")}>
      <TargetMenu
        actions={() => [
          {
            label: "Open in New Tab",
            run: () => openRouteOrSay(`/connections/${connection.id}`, { tab: true }),
          },
          {
            label: "Open in New Window",
            run: () => openRouteOrSay(`/connections/${connection.id}`, { tab: false }),
          },
          separator,
          {
            label: open ? "Fold" : "Unfold",
            run: toggle,
          },
          { label: "Collapse All", run: collapseAll },
          separator,
          ...menu.actions(),
        ]}
        render={
          <div
            ref={(node) => {
              draggable.setNodeRef(node)
              droppable.setNodeRef(node)
            }}
            {...draggable.listeners}
            data-sidebar-rowbox
            className={cn(
              "group/row flex items-start rounded-control",
              isSelected ? "bg-(--sel)" : "hover:bg-(--surf2)",
            )}
          />
        }
      >
        <button
          type="button"
          data-sidebar-toggle
          aria-expanded={open}
          aria-label={`${open ? "Fold" : "Unfold"} ${connection.name}`}
          onClick={toggle}
          className="mt-[7px] ml-1 flex size-4 shrink-0 items-center justify-center rounded text-(--text2) opacity-0 outline-none group-hover/row:opacity-100 focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-(--accent)/50"
        >
          <ChevronRightIcon
            aria-hidden="true"
            className={cn("size-3 transition-transform", open && "rotate-90")}
          />
        </button>
        <Link
          to="/connections/$connectionId"
          params={{ connectionId: connection.id }}
          data-sidebar-row
          aria-current={isSelected ? "page" : undefined}
          className={cn(
            "flex min-h-row-connection min-w-0 flex-1 items-center gap-2 rounded-control py-1 pr-2 pl-1 text-sm text-(--text) outline-none focus-visible:ring-2 focus-visible:ring-(--accent)/50",
            isSelected && "font-semibold",
          )}
        >
          {/* The name keeps its words whole (a short name never splits next to
              a long tag): when both do not fit, the tag wraps below it. */}
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-0.5">
            <span className="min-w-[4rem] flex-1 [overflow-wrap:anywhere]">{connection.name}</span>
            <EnvTag color={connection.envColor} label={connection.envLabel} />
          </span>
          {lockState === "locked" ? (
            <LockIcon
              role="img"
              aria-label="Read-only"
              className="size-3 shrink-0 text-(--text2)"
            />
          ) : null}
          {failures > 0 ? (
            <span
              className="shrink-0 font-tabular text-2xs text-(--state-failed-text)"
              title={`${failures} failed ${failures === 1 ? "job" : "jobs"}`}
              data-testid={`sidebar-failures-${connection.id}`}
            >
              {failures}
            </span>
          ) : null}
          <ReachabilityIndicator connectionId={connection.id} onlyProblems />
        </Link>
      </TargetMenu>
      {open ? <SidebarQueues connectionId={connection.id} queues={queues.data} /> : null}
      {menu.dialogs}
    </li>
  )
}
