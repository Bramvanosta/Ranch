import { useId, useState } from "react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Sheet } from "@/components/ui/sheet"
import { createEnvironment } from "@/data/environments/mutations"
import { createGroup } from "@/data/groups/mutations"
import { useInvalidateHierarchy } from "@/data/hierarchy"

import { EnvironmentColorPicker } from "./EnvironmentColorPicker"
import type { EnvColorToken } from "./palette"
import { defaultEnvNameForColor, shouldFollowColor } from "./placement"

interface NewPlacementDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /**
   * The group the environment is created in. Omit to also create the group
   * ("New group…" in the Move to menu): the dialog then asks for its name.
   */
  group?: { id: string; name: string }
  /** Called with the new environment's id, e.g. to move a connection into it. */
  onCreated?: (environmentId: string) => void
}

/**
 * "New environment" (group header menu) and the "New group… /
 * New environment…" entries of the Move to menu. Creates an empty
 * environment, optionally inside a brand-new group. A sheet.
 */
export function NewPlacementDialog({
  open,
  onOpenChange,
  group,
  onCreated,
}: NewPlacementDialogProps) {
  const invalidateHierarchy = useInvalidateHierarchy()
  const formId = useId()
  const [groupName, setGroupName] = useState("")
  const [envName, setEnvName] = useState("Local")
  const [color, setColor] = useState<EnvColorToken>("env-local")
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  function reset() {
    setGroupName("")
    setEnvName("Local")
    setColor("env-local")
    setError(null)
    setSaving(false)
  }

  function handleColor(next: EnvColorToken) {
    setColor(next)
    if (shouldFollowColor(envName)) setEnvName(defaultEnvNameForColor(next))
  }

  async function submit() {
    setError(null)
    if (!group && !groupName.trim()) {
      setError("Give the group a name")
      return
    }
    if (!envName.trim()) {
      setError("Give the environment a name")
      return
    }
    setSaving(true)

    let groupId = group?.id
    if (!groupId) {
      const created = await createGroup(groupName)
      if (created.status === "error") {
        setSaving(false)
        setError(created.error.message)
        return
      }
      groupId = created.data.id
    }

    const env = await createEnvironment({ groupId, name: envName, colorToken: color })
    // Refresh either way: the group may exist even if the environment failed.
    invalidateHierarchy()
    if (env.status === "error") {
      setSaving(false)
      setError(env.error.message)
      return
    }

    toast.success(`${envName.trim()} created`)
    onCreated?.(env.data)
    reset()
    onOpenChange(false)
  }

  const close = (next: boolean) => {
    if (!next) reset()
    onOpenChange(next)
  }

  return (
    <Sheet
      open={open}
      onOpenChange={close}
      title={group ? `New environment in ${group.name}` : "New group"}
      description={
        group
          ? "An environment holds the connections of one stage, such as Local, Staging or Production."
          : "A group usually maps to one project; it starts with a first environment."
      }
      testId="new-placement-sheet"
      footer={
        <>
          <Button type="button" variant="ghost" onClick={() => close(false)}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" form={formId} disabled={saving}>
            {saving ? "Creating" : "Create"}
          </Button>
        </>
      }
    >
      <form
        id={formId}
        className="grid gap-4"
        onSubmit={(e) => {
          e.preventDefault()
          void submit()
        }}
      >
        {group ? null : (
          <div className="grid gap-1.5">
            <Label htmlFor="new-placement-group">Group name</Label>
            <Input
              id="new-placement-group"
              value={groupName}
              onChange={(e) => setGroupName(e.target.value)}
              placeholder="e.g. Lumen"
              autoFocus
            />
          </div>
        )}
        <div className="grid gap-1.5">
          <Label htmlFor="new-placement-env">Environment name</Label>
          <Input
            id="new-placement-env"
            value={envName}
            onChange={(e) => setEnvName(e.target.value)}
            autoFocus={Boolean(group)}
          />
        </div>
        <div className="grid gap-1.5">
          <Label>Environment color</Label>
          <EnvironmentColorPicker value={color} onChange={handleColor} />
        </div>

        {error ? (
          <p role="alert" className="text-sm text-(--state-failed-text)">
            {error}
          </p>
        ) : null}
      </form>
    </Sheet>
  )
}
