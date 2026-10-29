import { zodResolver } from "@hookform/resolvers/zod"
import { LoaderCircleIcon, TriangleAlertIcon } from "lucide-react"
import { useId, useRef, useState } from "react"
import { useForm } from "react-hook-form"

import { Button } from "@/components/ui/button"
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form"
import { Input } from "@/components/ui/input"
import { Sheet } from "@/components/ui/sheet"
import {
  addConnection,
  setConnectionMcpReadonly,
  updateConnection,
} from "@/data/connections/mutations"
import { parseRedisUrl, testConnection, useBackends } from "@/data/connections/queries"
import { useEnvironments } from "@/data/environments/queries"
import { useMcpSafetySettings } from "@/data/mcp/queries"
import { useConnectionLockState } from "@/data/safety/queries"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import type { Connection, TestConnectionResult } from "@/lib/tauri/bindings"
import { SwitchRow } from "./ConnectionFormFields"
import { TestOutcome } from "./ConnectionTestOutcome"
import {
  type ConnectionFormInput,
  type ConnectionFormValues,
  connectionFormSchema,
} from "./connectionFormSchema"
import { type ConnectionForm, connectionForms, DEFAULT_BACKEND } from "./connectionForms"
import { PlacementFields } from "./PlacementFields"
import { NEW } from "./placement"

type ConnectionSheetProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  onSaved: (id: string) => void
} & (
  | {
      mode?: "create"
      /** Preselect a group / environment ("New connection here"). */
      preferredPlacement?: { groupId?: string; environmentId?: string }
      connection?: undefined
    }
  | { mode: "edit"; connection: Connection; preferredPlacement?: undefined }
)

/**
 * A connection in two steps, in a sheet. Step 1 is
 * where the server is: paste a URL or fill the fields, TLS and the SSH
 * tunnel behind disclosures, then "Test connection ↩". Step 2 shows what the
 * test found (or why it failed, with Back to fix it), then the name, the
 * placement and the protections; "Add connection" saves. Editing uses the
 * same two steps with the values filled.
 *
 * The passwords live only in react-hook-form state: sent on test and on
 * save, never stored client-side. Editing leaves them blank; a blank one
 * means the saved one, for the test as for the save.
 *
 * Step 1 is the form of the connection's backend kind (`connectionForms`);
 * a kind Ranch has no form for is said so, not edited.
 */
export function ConnectionSheet(props: ConnectionSheetProps) {
  // Mounted only while open: each opening starts again at step 1.
  if (!props.open) return null
  const kind = props.mode === "edit" ? props.connection.backend : DEFAULT_BACKEND
  const entry = connectionForms[kind]
  if (!entry) {
    return (
      <Sheet
        open
        onOpenChange={props.onOpenChange}
        title={props.mode === "edit" ? `Edit ${props.connection.name}` : "New connection"}
        description={`Ranch has no form for a ${kind} connection.`}
        footer={
          <Button type="button" variant="ghost" onClick={() => props.onOpenChange(false)}>
            Close
          </Button>
        }
        testId="connection-sheet"
        className="w-[min(40rem,calc(100vw-2rem))]"
      >
        {null}
      </Sheet>
    )
  }
  return <ConnectionSheetBody {...props} kind={kind} entry={entry} />
}

function ConnectionSheetBody(
  props: ConnectionSheetProps & { kind: string; entry: ConnectionForm },
) {
  const aiOn = useAiEnabled()
  const { onSaved, onOpenChange, kind, entry } = props
  const editing = props.mode === "edit" ? props.connection : null
  const isEdit = editing !== null
  const formId = useId()
  const urlRef = useRef<HTMLInputElement>(null)
  // What the backend lists for its endpoint: labels, required fields, defaults.
  const schema = useBackends().data?.find((b) => b.kind === kind)?.endpointSchema

  const form = useForm<ConnectionFormInput, undefined, ConnectionFormValues>({
    resolver: zodResolver(connectionFormSchema),
    defaultValues: entry.defaults(editing, schema),
  })

  const [step, setStep] = useState<"server" | "result">("server")
  const [urlInput, setUrlInput] = useState("")
  const [urlError, setUrlError] = useState<string | null>(null)
  const [testResult, setTestResult] = useState<TestConnectionResult | null>(null)
  const [testing, setTesting] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [saveErrorDocUrl, setSaveErrorDocUrl] = useState<string | null>(null)

  // `null` = untouched: follow the default.
  const [mcpReadonly, setMcpReadonly] = useState<boolean | null>(editing?.mcpReadonly ?? null)
  const [locked, setLocked] = useState<boolean | null>(null)
  const mcpReadonlyDefault = useMcpSafetySettings().data?.readonlyDefault ?? false
  const { data: lockState } = useConnectionLockState(editing?.id ?? "", {
    enabled: editing !== null,
  })
  const { data: allEnvironments } = useEnvironments()
  const watchedEnvColor = form.watch("env_color")
  const watchedEnvId = form.watch("environment_id")
  const placedInProduction =
    (watchedEnvId && watchedEnvId !== NEW
      ? allEnvironments?.find((e) => e.id === watchedEnvId)?.colorToken
      : watchedEnvColor) === "env-production"
  const lockedShown =
    locked ??
    (editing && lockState ? lockState === "locked" : !editing ? placedInProduction : false)

  const host = form.watch("redis_host")
  const port = form.watch("redis_port")
  const canTest = host.length > 0 && port >= 1 && port <= 65535 && !testing

  async function handleParse(raw: string) {
    setUrlInput(raw)
    setUrlError(null)
    if (!raw.trim()) return
    const result = await parseRedisUrl(raw.trim())
    if (result.status === "ok") {
      const p = result.data
      form.setValue("redis_host", p.host, { shouldValidate: true })
      form.setValue("redis_port", p.port, { shouldValidate: true })
      form.setValue("redis_db", p.db, { shouldValidate: true })
      form.setValue("redis_username", p.username ?? "", { shouldValidate: true })
      form.setValue("redis_password", p.password ?? "", { shouldValidate: true })
      form.setValue("tls", p.tls, { shouldValidate: true })
    } else {
      setUrlError(result.error.message)
    }
  }

  async function handleTest() {
    if (!canTest) return
    if (!(await form.trigger([...entry.stepOneFields]))) return
    setStep("result")
    setTesting(true)
    setTestResult(null)
    const v = form.getValues()
    const result = await testConnection(entry.testArgs(v, editing?.id ?? null))
    setTesting(false)
    const outcome: TestConnectionResult =
      result.status === "ok"
        ? result.data
        : {
            success: false,
            redisVersion: null,
            error: { kind: "other", reason: result.error.message },
            sshHostKeyPinned: null,
          }
    setTestResult(outcome)
    // A name to start from: the host, until the user gives a better one.
    if (outcome.success && !form.getValues("name")) form.setValue("name", v.redis_host)
  }

  function back() {
    setStep("server")
    setTestResult(null)
    setSaveError(null)
  }

  const save = form.handleSubmit(
    async (values) => {
      setSaving(true)
      setSaveError(null)
      setSaveErrorDocUrl(null)
      // The whole endpoint: an edit replaces the saved one (turning SSH
      // off removes the tunnel and its {id}::ssh secret).
      const common = entry.saveFields(values, locked)
      const result = editing
        ? await updateConnection({ ...common, id: editing.id })
        : await addConnection(common)
      // The MCP flag has its own (MCP-forbidden) command, sent
      // only when the user touched the switch.
      if (result.status === "ok" && mcpReadonly !== (editing?.mcpReadonly ?? null)) {
        const flag = await setConnectionMcpReadonly({
          connectionId: result.data.id,
          mcpReadonly,
        })
        if (flag.status === "error") {
          setSaving(false)
          setSaveError(`Saved, but the agent read-only setting failed: ${flag.error.message}`)
          return
        }
      }
      setSaving(false)
      if (result.status === "ok") {
        onSaved(result.data.id)
      } else {
        setSaveError(result.error.message)
        if (result.error.kind === "keychainUnavailable") setSaveErrorDocUrl(result.error.docUrl)
      }
    },
    (errors) => {
      // A step-1 field is wrong (it can only be after a change): show it there.
      if (entry.stepOneFields.some((f) => f in errors)) back()
    },
  )

  const succeeded = step === "result" && testResult?.success === true
  const failed = step === "result" && testResult !== null && !testResult.success

  const footer =
    step === "server" ? (
      <>
        <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
          Cancel
        </Button>
        <Button type="submit" form={formId} variant="primary" disabled={!canTest}>
          Test connection
          <span aria-hidden="true" className="ml-1 opacity-70">
            ↩
          </span>
        </Button>
      </>
    ) : (
      <>
        <Button type="button" variant="ghost" onClick={back} disabled={saving}>
          Back
        </Button>
        {failed ? (
          <Button type="button" variant="primary" onClick={handleTest} disabled={!canTest}>
            Try again
          </Button>
        ) : (
          <Button type="submit" form={formId} variant="primary" disabled={!succeeded || saving}>
            {saving ? (
              <>
                <LoaderCircleIcon className="mr-1.5 size-4 animate-spin" aria-hidden="true" />
                Saving
              </>
            ) : isEdit ? (
              "Save changes"
            ) : (
              "Add connection"
            )}
          </Button>
        )}
      </>
    )

  return (
    <Sheet
      open
      onOpenChange={onOpenChange}
      title={isEdit ? `Edit ${editing.name}` : "New connection"}
      description={
        step === "server"
          ? isEdit
            ? "Step 1 of 2: the server. Leave a password blank to keep the saved one."
            : "Step 1 of 2: the server. Paste a Redis URL or fill in the fields."
          : "Step 2 of 2: what Ranch found, and where the connection goes."
      }
      footer={footer}
      testId="connection-sheet"
      // A new connection starts with the URL; an edit, with the sheet itself.
      initialFocus={isEdit ? undefined : urlRef}
      className="w-[min(40rem,calc(100vw-2rem))]"
    >
      <Form {...form}>
        <form
          id={formId}
          onSubmit={(e) => {
            if (step === "server") {
              e.preventDefault()
              void handleTest()
            } else if (succeeded) {
              void save(e)
            } else {
              e.preventDefault()
            }
          }}
          className="flex flex-col gap-4"
        >
          {step === "server" ? (
            <entry.ServerStep
              form={form}
              urlRef={urlRef}
              isEdit={isEdit}
              saved={editing ? entry.saved(editing) : null}
              schema={schema}
              urlInput={urlInput}
              urlError={urlError}
              onUrl={handleParse}
            />
          ) : (
            <>
              <TestOutcome
                testing={testing}
                result={testResult}
                target={{ host, port }}
                tls={form.getValues("tls")}
              />
              {succeeded ? (
                <>
                  <FormField
                    control={form.control}
                    name="name"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Connection name</FormLabel>
                        <FormControl>
                          <Input placeholder="My local Redis" autoComplete="off" {...field} />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                  <PlacementFields
                    form={form}
                    isEdit={isEdit}
                    preferred={props.preferredPlacement}
                  />
                  <SwitchRow
                    id="connection-locked"
                    label="Read-only until I unlock it"
                    hint={
                      lockedShown
                        ? locked === null && placedInProduction && !isEdit
                          ? "On by default for Production. Writes need an unlock first, and it lasts until you lock it again."
                          : "Writes need an unlock first, and it lasts until you lock it again."
                        : "Ranch allows writes without an unlock. Destructive actions still ask."
                    }
                    checked={lockedShown}
                    onCheckedChange={setLocked}
                  />
                  {/* Reads are always allowed; no agents on the server image. */}
                  {aiOn ? (
                    <SwitchRow
                      id="connection-mcp-readonly"
                      label="Agents (MCP) can only read this connection"
                      hint={
                        mcpReadonly === null
                          ? placedInProduction
                            ? "Production environments are read-only for agents unless you turn this off."
                            : "Follows the default from Agents › Setup & safety until you change it."
                          : "Set for this connection. Writes that are allowed still ask for your approval."
                      }
                      checked={mcpReadonly ?? (placedInProduction || mcpReadonlyDefault)}
                      onCheckedChange={setMcpReadonly}
                    />
                  ) : null}
                </>
              ) : null}
              {saveError ? (
                <div className="flex items-start gap-2 rounded-md border border-(--danger)/40 bg-(--danger)/10 px-3 py-2 text-sm text-(--state-failed-text)">
                  <TriangleAlertIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                  <div className="min-w-0 flex-1 break-words">
                    <p>{saveError}</p>
                    {saveErrorDocUrl ? (
                      <a
                        href={saveErrorDocUrl}
                        target="_blank"
                        rel="noreferrer"
                        className="mt-1 inline-block underline underline-offset-2"
                      >
                        How to set up the keychain
                      </a>
                    ) : null}
                  </div>
                </div>
              ) : null}
            </>
          )}
        </form>
      </Form>
    </Sheet>
  )
}
