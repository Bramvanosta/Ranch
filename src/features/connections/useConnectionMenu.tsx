import { type ReactNode, useState } from "react"

import { type MenuAction, separator } from "@/components/menus/TargetMenu"
import {
  useDuplicateConnection,
  useMoveConnection,
  useMoveToGroup,
} from "@/data/connections/mutations"
import { useChangeEnvironmentColor } from "@/data/environments/mutations"
import { useEnvironments } from "@/data/environments/queries"
import { useGroups } from "@/data/groups/queries"
import { redisUrl } from "@/domain/bullmqEndpoint"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import type { Connection } from "@/lib/tauri/bindings"
import { EditConnectionDialog } from "./EditConnectionDialog"
import { NewPlacementDialog } from "./NewPlacementDialog"
import { ENV_COLOR_PALETTE } from "./palette"
import { environmentsOfGroup } from "./placement"
import { RemoveConnectionAlertDialog } from "./RemoveConnectionAlertDialog"
import { RenameEnvironmentSheet } from "./RenameEnvironmentSheet"

/**
 * What can be done to a connection (Edit, Duplicate, Environment, Move to
 * Group, Environment Color, Remove) in one place, so the sidebar row and the home
 * page card offer exactly the same actions, with the same
 * dialogs and the same toasts.
 *
 *   - `actions` is the right-click menu, for `<TargetMenu>`;
 *   - `dialogs` is rendered once next to the trigger;
 *   - `openEdit` / `openRemove` / `duplicate` are for visible buttons.
 */
export interface ConnectionMenu {
  /** The right-click menu, for `TargetMenu`. */
  actions: () => MenuAction[]
  dialogs: ReactNode
  openEdit: () => void
  openRemove: () => void
  duplicate: () => void
  isDuplicating: boolean
}

export function useConnectionMenu(connection: Connection): ConnectionMenu {
  const [editOpen, setEditOpen] = useState(false)
  const [removeOpen, setRemoveOpen] = useState(false)
  // "Move to… → New environment… / New group…": create, then move.
  const [newPlacement, setNewPlacement] = useState<
    { open: true; group?: { id: string; name: string } } | { open: false }
  >({ open: false })
  const moveConnection = useMoveConnection()
  const moveToGroup = useMoveToGroup()
  const [renaming, setRenaming] = useState<{
    id: string
    label: string
    groupName: string
  } | null>(null)

  function moveTo(environmentId: string) {
    moveConnection.mutate({ connectionId: connection.id, environmentId, name: connection.name })
  }

  const duplicateMutation = useDuplicateConnection()
  const changeColorMutation = useChangeEnvironmentColor()

  // The menu as data: a native menu on the desktop, the
  // in-window one in a browser (`TargetMenu`).
  const { data: groups = [] } = useGroups()
  const { data: environments = [] } = useEnvironments()
  const copy = useCopyToClipboard()
  const actions = (): MenuAction[] => [
    { label: "Edit Connection…", run: () => setEditOpen(true) },
    {
      label: "Duplicate",
      run: () => duplicateMutation.mutate(connection.id),
      disabled: duplicateMutation.isPending,
    },
    {
      label: "Copy Redis URL",
      run: () => {
        const url = redisUrl(connection)
        if (url) copy(url, "Redis URL")
      },
      disabled: redisUrl(connection) === null,
    },
    separator,
    {
      // The environment is a tag: retag within the group.
      kind: "submenu",
      label: "Environment",
      items: [
        ...environmentsOfGroup(environments, connection.groupId).map(
          (env): MenuAction => ({
            label: env.id === connection.environmentId ? `${env.label} (current)` : env.label,
            swatch: env.colorToken,
            disabled: env.id === connection.environmentId,
            run: () => moveTo(env.id),
          }),
        ),
        separator,
        {
          label: `Rename ${connection.envLabel}…`,
          run: () =>
            setRenaming({
              id: connection.environmentId,
              label: connection.envLabel,
              groupName: connection.groupName,
            }),
        },
        {
          label: "New Environment…",
          run: () =>
            setNewPlacement({
              open: true,
              group: { id: connection.groupId, name: connection.groupName },
            }),
        },
      ],
    },
    {
      // Another group, keeping the environment tag; the keyboard path of dragging.
      kind: "submenu",
      label: "Move to Group",
      items: [
        ...groups.map(
          (group): MenuAction => ({
            label: group.id === connection.groupId ? `${group.name} (current)` : group.name,
            disabled: group.id === connection.groupId,
            run: () => void moveToGroup(connection, group.id, environments),
          }),
        ),
        separator,
        { label: "New Group…", run: () => setNewPlacement({ open: true }) },
      ],
    },
    {
      kind: "submenu",
      label: "Environment Color",
      items: ENV_COLOR_PALETTE.map(
        (opt): MenuAction => ({
          label: connection.envColor === opt.token ? `${opt.name} (current)` : opt.name,
          swatch: opt.token,
          disabled: changeColorMutation.isPending,
          run: () =>
            changeColorMutation.mutate({
              environmentId: connection.environmentId,
              colorToken: opt.token,
            }),
        }),
      ),
    },
    separator,
    { label: "Remove Connection…", danger: true, run: () => setRemoveOpen(true) },
  ]

  const dialogs = (
    <>
      <EditConnectionDialog connection={connection} open={editOpen} onOpenChange={setEditOpen} />
      <RemoveConnectionAlertDialog
        connection={connection}
        open={removeOpen}
        onOpenChange={setRemoveOpen}
      />
      <RenameEnvironmentSheet
        environment={renaming}
        onOpenChange={(open) => {
          if (!open) setRenaming(null)
        }}
      />
      <NewPlacementDialog
        open={newPlacement.open}
        onOpenChange={(open) => {
          if (!open) setNewPlacement({ open: false })
        }}
        group={newPlacement.open ? newPlacement.group : undefined}
        onCreated={moveTo}
      />
    </>
  )

  return {
    actions,
    dialogs,
    openEdit: () => setEditOpen(true),
    openRemove: () => setRemoveOpen(true),
    duplicate: () => duplicateMutation.mutate(connection.id),
    isDuplicating: duplicateMutation.isPending,
  }
}
