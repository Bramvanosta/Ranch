import {
  type CollisionDetection,
  closestCenter,
  DndContext,
  type DragEndEvent,
  KeyboardSensor,
  type Modifier,
  PointerSensor,
  useSensor,
  useSensors,
} from "@dnd-kit/core"
import { restrictToParentElement, restrictToVerticalAxis } from "@dnd-kit/modifiers"
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { DatabaseIcon, FolderPlusIcon, PlusIcon } from "lucide-react"
import { useCallback, useEffect } from "react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent } from "@/components/ui/collapsible"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import { useMoveToGroup } from "@/data/connections/mutations"
import { useConnections } from "@/data/connections/queries"
import { useEnvironments } from "@/data/environments/queries"
import { useCreateGroup, useReorderGroups, useSetGroupCollapsed } from "@/data/groups/mutations"
import { useGroups } from "@/data/groups/queries"
import { useInvalidateHierarchy } from "@/data/hierarchy"
import type { Group } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { useSidebarFilterStore } from "@/stores/useSidebarFilterStore"
import { useSidebarUiStore } from "@/stores/useSidebarUiStore"
import { altArrowDelta, moveItemInList } from "./altArrowReorder"
import { ConnectionContextMenu } from "./ConnectionContextMenu"
import { ConnectionSheet } from "./ConnectionSheet"
import { GroupHeader } from "./GroupHeader"
import { GroupNameInput } from "./GroupNameInput"
import { SidebarFilterResults } from "./SidebarFilterView"
import { accepts, resolveDrop, type SidebarDndData } from "./sidebarDrop"
import { buildSidebarTree, type GroupNode } from "./treeShape"

/**
 * Only droppables the dragged kind may land on take part in collision
 * detection: a group lands on groups, a connection on a group or a connection.
 */
const typedClosestCenter: CollisionDetection = (args) => {
  const activeType = (args.active.data.current as SidebarDndData | undefined)?.type
  if (!activeType) return closestCenter(args)
  return closestCenter({
    ...args,
    droppableContainers: args.droppableContainers.filter((c) => {
      const overType = (c.data.current as SidebarDndData | undefined)?.type
      return overType ? accepts(activeType, overType) : false
    }),
  })
}

/** Groups reorder inside their list; a connection travels across the tree. */
const restrictSortablesToParent: Modifier = (args) => {
  const type = (args.active?.data.current as SidebarDndData | undefined)?.type
  return type === "connection" ? args.transform : restrictToParentElement(args)
}

/**
 * Connection sidebar: groups, their connections, the queues of an unfolded
 * connection. The environment is a tag on the connection (its menu changes
 * it), not a level of the tree. Groups reorder by dragging or Alt+arrow; a
 * connection dragged onto another group moves there with its tag. Intents
 * from context menus and the cmd-K palette arrive through `useSidebarUiStore`.
 */
export function ConnectionSidebar() {
  const createGroup = useCreateGroup()
  const invalidateHierarchy = useInvalidateHierarchy()
  const moveToGroup = useMoveToGroup()
  const setCollapsed = useSetGroupCollapsed()

  // Intents raised elsewhere (group / environment menus, cmd-K palette).
  const addConnectionTarget = useSidebarUiStore((s) => s.addConnectionTarget)
  const openAddConnection = useSidebarUiStore((s) => s.openAddConnection)
  const closeAddConnection = useSidebarUiStore((s) => s.closeAddConnection)
  const creatingGroup = useSidebarUiStore((s) => s.newGroupRequested)
  const requestNewGroup = useSidebarUiStore((s) => s.requestNewGroup)
  const clearNewGroupRequest = useSidebarUiStore((s) => s.clearNewGroupRequest)
  const revealGroupId = useSidebarUiStore((s) => s.revealGroupId)

  const { data: connections = [], isLoading, isError } = useConnections()

  const { data: groupRows = [], isLoading: groupsLoading, isError: groupsError } = useGroups()
  // Environments are an enrichment (empty ones, fresh label / color): if the
  // query fails the tree still renders from `connections`.
  const { data: environmentRows = [] } = useEnvironments()

  const groups = buildSidebarTree(groupRows, connections, environmentRows)
  // While the filter holds text, the matches replace the tree
  // (folding is left as it was, so clearing the filter restores it).
  const filtering = useSidebarFilterStore((s) => s.query.trim() !== "")

  // "Go to group" must be able to open a collapsed group; the header then
  // scrolls into view and takes focus (see `GroupHeader`).
  useEffect(() => {
    if (!revealGroupId) return
    const target = groupRows.find((g) => g.id === revealGroupId)
    if (target?.collapsed) setCollapsed.mutate({ id: target.id, collapsed: false })
  }, [revealGroupId, groupRows, setCollapsed])

  const reorderGroupsMutation = useReorderGroups()

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )

  const reorderGroups = useCallback(
    (activeId: string, delta: number) => {
      const idx = groups.findIndex((g) => g.id === activeId)
      const newIdx = idx + delta
      if (idx < 0 || newIdx < 0 || newIdx >= groups.length) return
      reorderGroupsMutation.mutate(moveItemInList(groups, idx, delta).map((g) => g.id))
    },
    [groups, reorderGroupsMutation],
  )

  function handleDragEnd(event: DragEndEvent) {
    const action = resolveDrop(
      event.active.data.current as SidebarDndData | undefined,
      event.over?.data.current as SidebarDndData | undefined,
    )
    if (!action) return

    if (action.kind === "reorderGroups") {
      const from = groups.findIndex((g) => g.id === action.activeId)
      const to = groups.findIndex((g) => g.id === action.overId)
      if (from >= 0 && to >= 0) reorderGroups(action.activeId, to - from)
      return
    }
    const connection = connections.find((c) => c.id === action.connectionId)
    if (connection) void moveToGroup(connection, action.groupId, environmentRows)
  }

  function handleSaved(_id: string) {
    // A save can create a group and an environment, and always changes counts.
    invalidateHierarchy()
    closeAddConnection()
    toast.success("Connection saved")
  }

  function handleCreateGroup(name: string) {
    clearNewGroupRequest()
    createGroup.mutate(name)
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-(--line) px-3 py-2">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-(--text2)">
          Connections
        </h2>
        <div className="flex items-center gap-0.5">
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6"
            onClick={requestNewGroup}
            aria-label="New group"
            title="New group"
          >
            <FolderPlusIcon className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6"
            onClick={() => openAddConnection()}
            aria-label="Add connection"
            title="Add connection"
          >
            <PlusIcon className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {/* Right-click on the sidebar background. */}
      <ContextMenu>
        <ContextMenuTrigger render={<div className="flex-1 overflow-y-auto py-1" />}>
          {filtering ? (
            <SidebarFilterResults />
          ) : isLoading || groupsLoading ? (
            <p className="px-3 py-4 text-xs text-(--text2)">Loading…</p>
          ) : isError || groupsError ? (
            <p className="px-3 py-4 text-xs text-(--state-failed-text)">
              Failed to load connections
            </p>
          ) : groups.length === 0 && !creatingGroup ? (
            <EmptyState onAdd={() => openAddConnection()} />
          ) : (
            <DndContext
              sensors={sensors}
              collisionDetection={typedClosestCenter}
              modifiers={[restrictToVerticalAxis, restrictSortablesToParent]}
              onDragEnd={handleDragEnd}
            >
              <SortableContext
                items={groups.map((g) => `group:${g.id}`)}
                strategy={verticalListSortingStrategy}
              >
                <ul className="flex flex-col gap-0.5 px-1">
                  {groups.map((group) => (
                    <GroupNodeView
                      key={group.id}
                      group={group}
                      allGroups={groupRows}
                      onGroupAltArrow={reorderGroups}
                    />
                  ))}
                  {creatingGroup ? (
                    <li className="flex items-center gap-1.5 px-2 py-1">
                      <FolderPlusIcon className="h-3.5 w-3.5 shrink-0 text-(--text2)" />
                      <GroupNameInput
                        groups={groupRows}
                        ariaLabel="New group name"
                        onSubmit={handleCreateGroup}
                        onCancel={clearNewGroupRequest}
                      />
                    </li>
                  ) : null}
                </ul>
              </SortableContext>
            </DndContext>
          )}
        </ContextMenuTrigger>
        <ContextMenuContent>
          <ContextMenuItem onClick={requestNewGroup}>New group</ContextMenuItem>
          <ContextMenuItem onClick={() => openAddConnection()}>Add connection</ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>

      {/* This pane is the connections explorer, nothing else: the Audit log
          link lives in the sidebar navigation above it. */}

      <ConnectionSheet
        open={addConnectionTarget !== null}
        onOpenChange={(open) => {
          if (!open) closeAddConnection()
        }}
        preferredPlacement={addConnectionTarget ?? undefined}
        onSaved={handleSaved}
      />
    </div>
  )
}

interface GroupNodeViewProps {
  group: GroupNode
  allGroups: Group[]
  onGroupAltArrow: (groupId: string, delta: -1 | 1) => void
}

function GroupNodeView({ group, allGroups, onGroupAltArrow }: GroupNodeViewProps) {
  const open = !group.collapsed
  const setCollapsed = useSetGroupCollapsed()
  const dndData: SidebarDndData = { type: "group", groupId: group.id }
  const sortable = useSortable({ id: `group:${group.id}`, data: dndData })
  const style = {
    transform: CSS.Transform.toString(sortable.transform),
    transition: sortable.transition,
    opacity: sortable.isDragging ? 0.5 : 1,
  }

  function handleHeaderKeyDown(e: React.KeyboardEvent) {
    const delta = altArrowDelta(e)
    if (delta !== null) {
      e.preventDefault()
      onGroupAltArrow(group.id, delta)
    }
  }

  return (
    <li
      ref={sortable.setNodeRef}
      style={style}
      {...sortable.attributes}
      data-sidebar-item
      className={cn(sortable.isOver && "rounded-md ring-1 ring-(--accent)/40")}
    >
      <Collapsible
        open={open}
        onOpenChange={(next) => setCollapsed.mutate({ id: group.id, collapsed: !next })}
      >
        <GroupHeader
          groupId={group.id}
          name={group.name}
          open={open}
          connections={group.connections}
          allGroups={allGroups}
          dragHandleProps={sortable.listeners}
          onToggleKeyDown={handleHeaderKeyDown}
        />
        <CollapsibleContent>
          {group.connections.length === 0 ? (
            <p className="ml-7 px-2 py-1 text-xs text-(--text2)">No connections yet</p>
          ) : (
            <ul className="ml-2 mt-0.5 flex flex-col gap-0.5">
              {group.connections.map((c) => (
                <ConnectionContextMenu key={c.id} connection={c} />
              ))}
            </ul>
          )}
        </CollapsibleContent>
      </Collapsible>
    </li>
  )
}

// `ConnectionRow` is folded into `ConnectionContextMenu`:
// base-ui's `<ContextMenuTrigger render={<li />}>` makes the trigger BE the
// `<li>` (vs. Radix's asChild that wraps an existing one). Keeping them
// separate would require an extra wrapper element.

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="flex flex-col items-center gap-2 px-4 py-8 text-center text-xs text-(--text2)">
      <DatabaseIcon className="h-6 w-6 opacity-60" />
      <p>No connections yet.</p>
      <Button size="sm" variant="default" onClick={onAdd} className="mt-1">
        <PlusIcon className="mr-1.5 h-3.5 w-3.5" />
        Add your first
      </Button>
    </div>
  )
}
