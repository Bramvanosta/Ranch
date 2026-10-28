/**
 * What a drag in the sidebar means (pure, unit-tested).
 *
 * One `DndContext` carries two kinds of draggables. Each node registers
 * `data: SidebarDndData`; collision detection only considers droppables a
 * given draggable may land on (`accepts`), and `resolveDrop` turns the final
 * (active, over) pair into an action.
 *
 *   group      → reorder among groups
 *   connection → move to another group, keeping its environment tag: dropped
 *                on the group's header or on one of its connections
 *
 * The environment is a tag, changed from the connection's menu; within a
 * group connections are ordered by environment, then name.
 */
export type SidebarDndData =
  | { type: "group"; groupId: string }
  | { type: "connection"; connectionId: string; groupId: string }

export type SidebarDropAction =
  | { kind: "reorderGroups"; activeId: string; overId: string }
  | { kind: "moveToGroup"; connectionId: string; groupId: string }
  | null

export function accepts(active: SidebarDndData["type"], over: SidebarDndData["type"]): boolean {
  if (active === "group") return over === "group"
  return true // a connection may land on a group header or on a connection
}

export function resolveDrop(
  active: SidebarDndData | undefined,
  over: SidebarDndData | undefined,
): SidebarDropAction {
  if (!active || !over) return null

  if (active.type === "group") {
    if (over.type !== "group" || over.groupId === active.groupId) return null
    return { kind: "reorderGroups", activeId: active.groupId, overId: over.groupId }
  }

  if (over.groupId === active.groupId) return null
  return { kind: "moveToGroup", connectionId: active.connectionId, groupId: over.groupId }
}
