import { useEffect, useId, useState } from "react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Sheet } from "@/components/ui/sheet"
import { useInvalidateHierarchy } from "@/features/groups/useGroups"
import { commands } from "@/lib/tauri/bindings"

/**
 * Rename an environment tag ("Environment › Rename …" on a connection): every
 * connection carrying it in the group shows the new name.
 */
export function RenameEnvironmentSheet({
  environment,
  onOpenChange,
}: {
  /** The tag to rename; `null` = closed. */
  environment: { id: string; label: string; groupName: string } | null
  onOpenChange: (open: boolean) => void
}) {
  const invalidateHierarchy = useInvalidateHierarchy()
  const formId = useId()
  const [label, setLabel] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    setLabel(environment?.label ?? "")
    setError(null)
    setSaving(false)
  }, [environment])

  async function submit() {
    if (!environment) return
    const next = label.trim()
    if (!next) {
      setError("Give the environment a name")
      return
    }
    if (next === environment.label) {
      onOpenChange(false)
      return
    }
    setSaving(true)
    const r = await commands.renameEnvironment({ environmentId: environment.id, newLabel: next })
    invalidateHierarchy()
    if (r.status === "error") {
      setSaving(false)
      setError(r.error.message)
      return
    }
    toast.success(`${environment.label} renamed to ${next}`)
    onOpenChange(false)
  }

  return (
    <Sheet
      open={environment !== null}
      onOpenChange={onOpenChange}
      title={environment ? `Rename ${environment.label}` : "Rename environment"}
      description={
        environment
          ? `Every connection of ${environment.groupName} tagged ${environment.label} takes the new name.`
          : undefined
      }
      testId="rename-environment-sheet"
      footer={
        <>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" form={formId} disabled={saving}>
            {saving ? "Renaming" : "Rename"}
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
        <div className="grid gap-1.5">
          <Label htmlFor="rename-environment">Environment name</Label>
          <Input
            id="rename-environment"
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            autoFocus
          />
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
