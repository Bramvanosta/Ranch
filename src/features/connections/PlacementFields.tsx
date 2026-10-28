import { useEffect, useRef } from "react"
import type { UseFormReturn } from "react-hook-form"

import { EnvDot } from "@/components/data/Env"
import { FormControl, FormField, FormItem, FormLabel, FormMessage } from "@/components/ui/form"
import { Input } from "@/components/ui/input"
import { useEnvironments } from "@/data/environments/queries"
import { useGroups } from "@/data/groups/queries"
import { cn } from "@/lib/utils"
import type { ConnectionFormInput, ConnectionFormValues } from "./connectionFormSchema"
import { EnvironmentColorPicker } from "./EnvironmentColorPicker"
import type { EnvColorToken } from "./palette"
import {
  defaultEnvNameForColor,
  environmentsOfGroup,
  initialPlacement,
  NEW,
  shouldFollowColor,
} from "./placement"

const selectClassName = cn(
  "h-9 w-full rounded-md border border-(--line) bg-transparent px-3 text-sm shadow-xs outline-none",
  "focus-visible:border-(--accent) focus-visible:ring-[3px] focus-visible:ring-(--accent)/50",
  "disabled:cursor-not-allowed disabled:opacity-50",
)

interface PlacementFieldsProps {
  form: UseFormReturn<ConnectionFormInput, undefined, ConnectionFormValues>
  /** Create mode only: preselect this group / environment ("New connection here"). */
  preferred?: { groupId?: string; environmentId?: string }
  /** Edit mode: placement comes from the saved connection, never from defaults. */
  isEdit: boolean
}

/**
 * Group + Environment fields of `ConnectionForm`.
 *
 * Both are a select of what exists plus a "New …" entry that reveals a name
 * field (and, for an environment, the color picker). The color belongs to the
 * environment: for an existing one it is shown read-only, and it is never
 * used to decide where the connection goes.
 */
export function PlacementFields({ form, preferred, isEdit }: PlacementFieldsProps) {
  // A failed load must not leave the form stuck on "Loading…": treat it as
  // "nothing exists" so the user can still create a group + environment.
  const groupsQuery = useGroups()
  const envsQuery = useEnvironments()
  const groups = groupsQuery.data
  const environments = envsQuery.data
  const groupsReady = groupsQuery.isSuccess || groupsQuery.isError
  const envsReady = envsQuery.isSuccess || envsQuery.isError

  const groupId = form.watch("group_id")
  const environmentId = form.watch("environment_id")
  const envColor = form.watch("env_color") as EnvColorToken

  const groupList = groups ?? []
  const envList = groupId && groupId !== NEW ? environmentsOfGroup(environments ?? [], groupId) : []
  const groupIsNew = groupId === NEW
  const envIsNew = groupIsNew || environmentId === NEW
  const selectedEnv = envList.find((e) => e.id === environmentId)

  // Create mode: once both lists are known, fill the placement the user has
  // not touched yet. Runs once; later edits belong to the user.
  const initialised = useRef(false)
  useEffect(() => {
    if (isEdit || initialised.current || !groupsReady || !envsReady) return
    initialised.current = true
    const initial = initialPlacement(groups ?? [], environments ?? [], preferred)
    form.setValue("group_id", initial.group_id)
    form.setValue("group_name", initial.group_name)
    form.setValue("environment_id", initial.environment_id)
    form.setValue("environment_name", initial.environment_name)
  }, [isEdit, groupsReady, envsReady, groups, environments, preferred, form])

  function handleGroupChange(next: string) {
    form.setValue("group_id", next, { shouldValidate: true })
    if (next === NEW) {
      form.setValue("environment_id", NEW)
      if (!form.getValues("environment_name")) {
        form.setValue("environment_name", defaultEnvNameForColor(envColor))
      }
      return
    }
    const envs = environmentsOfGroup(environments ?? [], next)
    form.setValue("environment_id", envs[0]?.id ?? NEW)
    if (envs.length === 0 && !form.getValues("environment_name")) {
      form.setValue("environment_name", defaultEnvNameForColor(envColor))
    }
  }

  function handleEnvironmentChange(next: string) {
    form.setValue("environment_id", next, { shouldValidate: true })
    if (next === NEW && !form.getValues("environment_name")) {
      form.setValue("environment_name", defaultEnvNameForColor(envColor))
    }
  }

  function handleColorChange(next: EnvColorToken) {
    form.setValue("env_color", next)
    if (shouldFollowColor(form.getValues("environment_name") ?? "")) {
      form.setValue("environment_name", defaultEnvNameForColor(next))
    }
  }

  return (
    <fieldset className="grid gap-4 rounded-md border border-(--line) p-3">
      <legend className="px-1 text-xs font-medium text-(--text2)">Placement</legend>

      <div className="grid gap-4 sm:grid-cols-2">
        <FormField
          control={form.control}
          name="group_id"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Group</FormLabel>
              <FormControl>
                <select
                  {...field}
                  onChange={(e) => handleGroupChange(e.target.value)}
                  className={selectClassName}
                >
                  {field.value === "" ? <option value="">Loading…</option> : null}
                  {groupList.map((g) => (
                    <option key={g.id} value={g.id}>
                      {g.name}
                    </option>
                  ))}
                  <option value={NEW}>New group…</option>
                </select>
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />

        <FormField
          control={form.control}
          name="environment_id"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Environment</FormLabel>
              {/* `FormControl` must wrap the <select> itself: it carries the id
                  the <label> points at. The swatch sits beside it. */}
              <div className="flex items-center gap-2">
                {selectedEnv ? (
                  <EnvDot
                    color={selectedEnv.colorToken}
                    decorative
                    data-testid="existing-env-color"
                  />
                ) : null}
                <FormControl>
                  <select
                    {...field}
                    value={groupIsNew ? NEW : field.value}
                    disabled={groupIsNew}
                    onChange={(e) => handleEnvironmentChange(e.target.value)}
                    className={selectClassName}
                  >
                    {envList.map((e) => (
                      <option key={e.id} value={e.id}>
                        {e.label}
                      </option>
                    ))}
                    <option value={NEW}>New environment…</option>
                  </select>
                </FormControl>
              </div>
              <FormMessage />
            </FormItem>
          )}
        />
      </div>

      {groupIsNew ? (
        <FormField
          control={form.control}
          name="group_name"
          render={({ field }) => (
            <FormItem>
              <FormLabel>New group name</FormLabel>
              <FormControl>
                <Input placeholder="e.g. Lumen" {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      ) : null}

      {envIsNew ? (
        <>
          <FormField
            control={form.control}
            name="environment_name"
            render={({ field }) => (
              <FormItem>
                <FormLabel>New environment name</FormLabel>
                <FormControl>
                  <Input placeholder="e.g. Staging" {...field} />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
          <FormField
            control={form.control}
            name="env_color"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Environment color</FormLabel>
                <FormControl>
                  <EnvironmentColorPicker
                    value={field.value as EnvColorToken}
                    onChange={handleColorChange}
                  />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
        </>
      ) : null}
    </fieldset>
  )
}
