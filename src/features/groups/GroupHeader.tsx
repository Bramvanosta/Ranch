import { ChevronRightIcon, FolderIcon } from "lucide-react"
import { useEffect, useRef, useState } from "react"
import { toast } from "sonner"

import { CollapsibleTrigger } from "@/components/ui/collapsible"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import { NewPlacementDialog } from "@/features/connections/NewPlacementDialog"
import type { Connection, Group } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { useSidebarUiStore } from "@/stores/useSidebarUiStore"

import { DeleteGroupDialog } from "./DeleteGroupDialog"
import { GroupNameInput } from "./GroupNameInput"
import { useGroupSummary } from "./groupSummary"
import { useDeleteGroup, useRenameGroup } from "./useGroups"

interface GroupHeaderProps {
  groupId: string
  name: string
  open: boolean
  /** Every connection under this group — drives the count and the delete confirmation. */
  connections: Connection[]
  /** All groups, for the duplicate-name check while renaming. */
  allGroups: Pick<Group, "id" | "name">[]
  /** dnd-kit listeners for the drag handle (mouse-driven reorder). */
  dragHandleProps?: Record<string, unknown>
  /** Alt+arrow reorder and any other header-level key handling owned by the sidebar. */
  onToggleKeyDown?: (e: React.KeyboardEvent) => void
}

/**
 * Header row of a group in the sidebar (UX: `GroupHeader`).
 *
 * Must be rendered inside a `<Collapsible>`: the chevron is its trigger.
 * Rename: `F2` on the chevron, double-click on the name, or the context menu.
 * Delete: immediate for an empty group, confirmation listing the connections
 * otherwise.
 */
export function GroupHeader({
  groupId,
  name,
  open,
  connections,
  allGroups,
  dragHandleProps,
  onToggleKeyDown,
}: GroupHeaderProps) {
  const [isRenaming, setIsRenaming] = useState(false)
  const [deleteOpen, setDeleteOpen] = useState(false)
  const [newEnvOpen, setNewEnvOpen] = useState(false)
  const renameGroup = useRenameGroup()
  const deleteGroup = useDeleteGroup()
  const openAddConnection = useSidebarUiStore((s) => s.openAddConnection)

  // cmd-K "Go to group" / "Rename group…": scroll to this header,
  // focus it, and optionally start the inline rename. (Expanding a collapsed
  // group is the sidebar's job — it owns the collapse mutation.)
  const revealGroupId = useSidebarUiStore((s) => s.revealGroupId)
  const renameGroupId = useSidebarUiStore((s) => s.renameGroupId)
  const clearReveal = useSidebarUiStore((s) => s.clearReveal)
  const toggleRef = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    if (revealGroupId !== groupId) return
    toggleRef.current?.scrollIntoView?.({ block: "nearest" })
    toggleRef.current?.focus()
    if (renameGroupId === groupId) setIsRenaming(true)
    clearReveal()
  }, [revealGroupId, renameGroupId, groupId, clearReveal])

  function handleRename(newName: string) {
    setIsRenaming(false)
    renameGroup.mutate(
      { id: groupId, name: newName },
      {
        onError: (err) =>
          toast.error(`Could not rename: ${err instanceof Error ? err.message : "unknown error"}`),
      },
    )
  }

  function handleDelete() {
    if (connections.length > 0) {
      setDeleteOpen(true)
      return
    }
    deleteGroup.mutate(
      { id: groupId, cascade: false },
      {
        onSuccess: () => toast.success(`${name} deleted`),
        onError: (err) =>
          toast.error(
            `Could not delete group: ${err instanceof Error ? err.message : "unknown error"}`,
          ),
      },
    )
  }

  return (
    <>
      <ContextMenu>
        <ContextMenuTrigger
          render={
            <div
              data-testid={`group-header-${groupId}`}
              data-sidebar-rowbox
              className="group/row flex min-h-row-queue w-full items-center gap-1.5 rounded-control px-1 text-2xs text-(--text2) hover:bg-(--surf2)"
            />
          }
        >
          <CollapsibleTrigger
            render={
              <button
                ref={toggleRef}
                type="button"
                data-sidebar-row
                data-sidebar-toggle
                aria-label={`Toggle ${name}`}
                onKeyDown={(e) => {
                  if (e.key === "F2") {
                    e.preventDefault()
                    setIsRenaming(true)
                    return
                  }
                  onToggleKeyDown?.(e)
                }}
                // The chevron shows on hover or keyboard focus only.
                className="flex size-4 shrink-0 items-center justify-center rounded opacity-0 outline-none group-hover/row:opacity-100 focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-(--accent)/50"
              >
                <ChevronRightIcon
                  aria-hidden="true"
                  className={cn("size-3 transition-transform", open && "rotate-90")}
                />
              </button>
            }
          />
          {/* Drag handle — mouse-driven only. Keyboard reorder is alt-arrow on the chevron. */}
          <span
            {...dragHandleProps}
            aria-hidden="true"
            className="flex h-3.5 w-3.5 shrink-0 cursor-grab items-center justify-center opacity-70 active:cursor-grabbing"
          >
            <FolderIcon className="h-3.5 w-3.5" />
          </span>
          {isRenaming ? (
            <GroupNameInput
              initialValue={name}
              groups={allGroups}
              excludeId={groupId}
              ariaLabel={`Rename group ${name}`}
              onSubmit={handleRename}
              onCancel={() => setIsRenaming(false)}
            />
          ) : (
            <button
              type="button"
              onDoubleClick={() => setIsRenaming(true)}
              title="Double-click to rename"
              className="min-w-0 flex-1 rounded text-left font-semibold tracking-wide break-words uppercase focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)/50"
            >
              {name}
            </button>
          )}
          <GroupCount open={open} connections={connections} />
        </ContextMenuTrigger>
        <ContextMenuContent>
          <ContextMenuItem onClick={() => openAddConnection({ groupId })}>
            New connection here
          </ContextMenuItem>
          <ContextMenuItem onClick={() => setNewEnvOpen(true)}>New environment</ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem onClick={() => setIsRenaming(true)}>Rename</ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem
            onClick={handleDelete}
            disabled={deleteGroup.isPending}
            className="text-(--state-failed-text) focus:text-(--state-failed-text)"
          >
            {connections.length > 0 ? "Delete…" : "Delete"}
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>

      <DeleteGroupDialog
        groupId={groupId}
        groupName={name}
        connections={connections}
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
      />
      <NewPlacementDialog
        open={newEnvOpen}
        onOpenChange={setNewEnvOpen}
        group={{ id: groupId, name }}
      />
    </>
  )
}

/** Open: how many connections. Folded: one word that says what is inside. */
function GroupCount({ open, connections }: { open: boolean; connections: Connection[] }) {
  const summary = useGroupSummary(connections)
  if (summary.kind === "empty") return <span className="pr-1 text-2xs">empty</span>
  if (!open && summary.kind === "offline") return <span className="pr-1 text-2xs">offline</span>
  return (
    <span className="flex items-center gap-1.5 pr-1">
      {!open && summary.kind === "count" && summary.failures > 0 ? (
        <span
          className="font-tabular text-2xs text-(--state-failed-text)"
          title={`${summary.failures} failed jobs`}
        >
          {summary.failures}
        </span>
      ) : null}
      <span className="font-tabular text-2xs">{connections.length}</span>
    </span>
  )
}
