import { useNavigate } from "@tanstack/react-router"
import { toast } from "sonner"
import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { EnvDot } from "@/components/data/Env"
import { useDeleteGroup } from "@/data/groups/mutations"
import { useSelectedConnectionId } from "@/hooks/useSelectedConnectionId"
import type { Connection } from "@/lib/tauri/bindings"

interface DeleteGroupDialogProps {
  groupId: string
  groupName: string
  /** Connections that will be removed with the group, listed so the blast radius is explicit. */
  connections: Pick<Connection, "id" | "name" | "envLabel" | "envColor">[]
  open: boolean
  onOpenChange: (open: boolean) => void
}

/**
 * Confirmation for deleting a group that still holds connections
 *. Empty groups are deleted without it. A destructive sheet like
 * `RemoveConnectionAlertDialog`: Cancel is the safe default.
 */
export function DeleteGroupDialog({
  groupId,
  groupName,
  connections,
  open,
  onOpenChange,
}: DeleteGroupDialogProps) {
  const navigate = useNavigate()
  const selectedId = useSelectedConnectionId()
  const deleteGroup = useDeleteGroup()

  function confirm() {
    deleteGroup.mutate(
      { id: groupId, cascade: true },
      {
        onSuccess: () => {
          toast.success(`${groupName} deleted`)
          if (selectedId && connections.some((c) => c.id === selectedId)) {
            navigate({ to: "/" })
          }
          onOpenChange(false)
        },
      },
    )
  }

  return (
    <ConfirmSheet
      open={open}
      onOpenChange={onOpenChange}
      danger
      title={`Delete ${groupName}?`}
      description={
        <>
          {connections.length === 1
            ? "This group, its connection and the stored credential will be permanently removed."
            : `This group, its ${connections.length} connections and their stored credentials will be permanently removed.`}{" "}
          This cannot be undone.
        </>
      }
      // Closed in onSuccess so the toast and the navigation fire in order.
      onConfirm={confirm}
      busy={deleteGroup.isPending}
      confirmLabel={deleteGroup.isPending ? "Deleting" : "Delete group"}
    >
      <ul
        aria-label="Connections that will be removed"
        className="max-h-40 overflow-y-auto rounded-card border border-(--line) px-2 py-1.5 text-sm"
      >
        {connections.map((c) => (
          <li key={c.id} className="flex items-start gap-2 py-0.5">
            <EnvDot color={c.envColor} decorative className="mt-1.5" />
            <span className="min-w-0 break-words">{c.name}</span>
            <span className="ml-auto shrink-0 text-xs text-(--text2)">{c.envLabel}</span>
          </li>
        ))}
      </ul>
    </ConfirmSheet>
  )
}
