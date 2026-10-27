import { zodResolver } from "@hookform/resolvers/zod"
import {
  ChevronRightIcon,
  CircleCheckIcon,
  LoaderCircleIcon,
  TriangleAlertIcon,
} from "lucide-react"
import { type ReactNode, type RefObject, useId, useRef, useState } from "react"
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
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { useMcpSafetySettings } from "@/features/mcp/useMcpSafety"
import { formatCount } from "@/features/metrics/format"
import { useConnectionLockState } from "@/features/safety/useConnectionLockState"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import {
  type Connection,
  type ConnectionProbe,
  commands,
  type TestConnectionResult,
} from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import {
  type ConnectionFormInput,
  type ConnectionFormValues,
  connectionFormSchema,
} from "./connectionFormSchema"
import { PlacementFields } from "./PlacementFields"
import type { EnvColorToken } from "./palette"
import { buildPlacementPayload, NEW } from "./placement"
import { plainTestError } from "./plainError"
import { buildSshTunnelPayload, parseSshTunnelDefaults } from "./sshTunnelPayload"
import { useEnvironments } from "./useEnvironments"

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

/** What step 1 holds: everything the test needs, nothing it does not. */
const STEP_ONE_FIELDS = [
  "redis_host",
  "redis_port",
  "redis_db",
  "redis_username",
  "redis_password",
  "tls",
  "tls_custom_ca",
  "ssh_enabled",
  "ssh_host",
  "ssh_port",
  "ssh_user",
  "ssh_auth_kind",
  "ssh_key_file_path",
  "ssh_password",
  "ssh_jump_enabled",
  "ssh_jump_host",
  "ssh_jump_port",
  "ssh_jump_user",
] as const satisfies ReadonlyArray<keyof ConnectionFormInput>

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
 */
export function ConnectionSheet(props: ConnectionSheetProps) {
  // Mounted only while open: each opening starts again at step 1.
  return props.open ? <ConnectionSheetBody {...props} /> : null
}

function ConnectionSheetBody(props: ConnectionSheetProps) {
  const aiOn = useAiEnabled()
  const { onSaved, onOpenChange } = props
  const editing = props.mode === "edit" ? props.connection : null
  const isEdit = editing !== null
  const formId = useId()
  const urlRef = useRef<HTMLInputElement>(null)

  // SSH defaults derive from the saved connection (edit) or the
  // "no SSH" baseline; the SSH password is never round-tripped.
  const sshDefaults = parseSshTunnelDefaults(editing)
  const form = useForm<ConnectionFormInput, undefined, ConnectionFormValues>({
    resolver: zodResolver(connectionFormSchema),
    defaultValues: editing
      ? {
          name: editing.name,
          redis_host: editing.host,
          redis_port: editing.port,
          redis_db: editing.db,
          redis_username: editing.username ?? "",
          redis_password: "",
          tls: editing.tls,
          tls_custom_ca: editing.tlsCustomCa ?? "",
          ...sshDefaults,
          ssh_password: "",
          group_id: editing.groupId,
          group_name: "",
          environment_id: editing.environmentId,
          environment_name: "",
          env_color: editing.envColor as EnvColorToken,
        }
      : {
          name: "",
          redis_host: "",
          redis_port: 6379,
          redis_db: 0,
          redis_username: "",
          redis_password: "",
          tls: false,
          tls_custom_ca: "",
          ...sshDefaults,
          ssh_password: "",
          group_id: "",
          group_name: "",
          environment_id: "",
          environment_name: "",
          env_color: "env-local",
        },
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
    const result = await commands.parseRedisUrl(raw.trim())
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
    if (!(await form.trigger([...STEP_ONE_FIELDS]))) return
    setStep("result")
    setTesting(true)
    setTestResult(null)
    const v = form.getValues()
    const result = await commands.testConnection({
      host: v.redis_host,
      port: v.redis_port,
      db: v.redis_db,
      username: v.redis_username || null,
      password: v.redis_password || null,
      tls: v.tls,
      // The PEM only matters with TLS on.
      tlsCustomCa: v.tls ? v.tls_custom_ca || null : null,
      sshTunnel: buildSshTunnelPayload(v),
      sshPassword:
        v.ssh_enabled && v.ssh_auth_kind === "password" && v.ssh_password ? v.ssh_password : null,
      savedConnectionId: editing?.id ?? null,
    })
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
      // An empty password is "keep the saved one" (edit) or "no auth" (create).
      const password = values.redis_password ? values.redis_password : null
      const tlsCustomCa = values.tls ? values.tls_custom_ca || null : null
      const sshTunnel = buildSshTunnelPayload(values)
      const sshPassword =
        sshTunnel && sshTunnel.auth.kind === "password" && values.ssh_password
          ? values.ssh_password
          : null
      const common = {
        name: values.name,
        host: values.redis_host,
        port: values.redis_port,
        db: values.redis_db,
        username: values.redis_username || null,
        password,
        tls: values.tls,
        tlsCustomCa,
        sshTunnel,
        sshPassword,
        // Untouched = the backend's default (read-only for Production).
        locked,
        ...buildPlacementPayload(values),
      }
      const result = editing
        ? await commands.updateConnection({
            ...common,
            id: editing.id,
            // Turning SSH off clears the column and the {id}::ssh secret.
            sshTunnelClear: Boolean(editing.sshTunnel) && !values.ssh_enabled,
          })
        : await commands.addConnection(common)
      // The MCP flag has its own (MCP-forbidden) command, sent
      // only when the user touched the switch.
      if (result.status === "ok" && mcpReadonly !== (editing?.mcpReadonly ?? null)) {
        const flag = await commands.setConnectionMcpReadonly({
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
      if (STEP_ONE_FIELDS.some((f) => f in errors)) back()
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
            <ServerStep
              form={form}
              urlRef={urlRef}
              isEdit={isEdit}
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
                            : "Follows the default from Settings › Agents › Safety until you change it."
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

type FormApi = ReturnType<typeof useForm<ConnectionFormInput, undefined, ConnectionFormValues>>

function NumberField({
  form,
  name,
  label,
  min,
  max,
}: {
  form: FormApi
  name: "redis_port" | "redis_db" | "ssh_port" | "ssh_jump_port"
  label: string
  min: number
  max: number
}) {
  return (
    <FormField
      control={form.control}
      name={name}
      render={({ field }) => (
        <FormItem>
          <FormLabel>{label}</FormLabel>
          <FormControl>
            <Input
              type="number"
              min={min}
              max={max}
              {...field}
              onChange={(e) => field.onChange(e.target.valueAsNumber)}
            />
          </FormControl>
          <FormMessage />
        </FormItem>
      )}
    />
  )
}

function TextField({
  form,
  name,
  label,
  placeholder,
  type,
}: {
  form: FormApi
  name:
    | "redis_host"
    | "redis_username"
    | "redis_password"
    | "ssh_host"
    | "ssh_user"
    | "ssh_key_file_path"
    | "ssh_password"
    | "ssh_jump_host"
    | "ssh_jump_user"
  label: string
  placeholder?: string
  type?: "password"
}) {
  return (
    <FormField
      control={form.control}
      name={name}
      render={({ field }) => (
        <FormItem>
          <FormLabel>{label}</FormLabel>
          <FormControl>
            <Input
              type={type}
              placeholder={placeholder}
              autoComplete={type === "password" ? "new-password" : "off"}
              spellCheck={false}
              {...field}
            />
          </FormControl>
          <FormMessage />
        </FormItem>
      )}
    />
  )
}

function CheckboxField({
  form,
  name,
  label,
}: {
  form: FormApi
  name: "tls" | "ssh_enabled" | "ssh_jump_enabled"
  label: string
}) {
  return (
    <FormField
      control={form.control}
      name={name}
      render={({ field }) => (
        <FormItem className="flex flex-row items-center gap-3">
          <FormControl>
            <input
              type="checkbox"
              checked={field.value}
              onChange={(e) => field.onChange(e.target.checked)}
              className="size-4 cursor-pointer"
            />
          </FormControl>
          <FormLabel className="!m-0 cursor-pointer text-sm font-normal">{label}</FormLabel>
        </FormItem>
      )}
    />
  )
}

/** A section folded by default unless it is in use. */
function Disclosure({
  title,
  status,
  defaultOpen,
  testId,
  children,
}: {
  title: string
  status: string
  defaultOpen: boolean
  testId: string
  children: ReactNode
}) {
  return (
    <details
      open={defaultOpen}
      data-testid={testId}
      className="group rounded-md border border-(--line)"
    >
      <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-sm font-medium [&::-webkit-details-marker]:hidden">
        <ChevronRightIcon
          className="size-4 shrink-0 text-(--text2) transition-transform group-open:rotate-90"
          aria-hidden="true"
        />
        {title}
        <span className="ml-auto text-xs font-normal text-(--text2)">{status}</span>
      </summary>
      <div className="flex flex-col gap-3 border-t border-(--line) px-3 py-3">{children}</div>
    </details>
  )
}

function ServerStep({
  form,
  urlRef,
  isEdit,
  urlInput,
  urlError,
  onUrl,
}: {
  form: FormApi
  urlRef: RefObject<HTMLInputElement | null>
  isEdit: boolean
  urlInput: string
  urlError: string | null
  onUrl: (raw: string) => void
}) {
  const tls = form.watch("tls")
  const ssh = form.watch("ssh_enabled")
  const sshHost = form.watch("ssh_host")
  const keep = isEdit ? "Leave blank to keep current" : undefined
  return (
    <>
      <div className="flex flex-col gap-1.5">
        <label htmlFor="connection-url" className="text-sm font-medium">
          Paste a Redis URL
        </label>
        <Input
          id="connection-url"
          ref={urlRef}
          placeholder="redis://default:secret@host:6379/0"
          value={urlInput}
          onChange={(e) => onUrl(e.target.value)}
          autoComplete="off"
          spellCheck={false}
        />
        {urlError ? (
          <p className="text-sm text-(--state-failed-text)">{urlError}</p>
        ) : (
          <p className="text-xs text-(--text2)">
            It fills the fields below; the password goes to the keychain on save.
          </p>
        )}
      </div>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-[minmax(0,1fr)_6rem_5rem]">
        <TextField form={form} name="redis_host" label="Host" placeholder="localhost" />
        <NumberField form={form} name="redis_port" label="Port" min={1} max={65535} />
        <NumberField form={form} name="redis_db" label="DB index" min={0} max={15} />
      </div>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <TextField
          form={form}
          name="redis_username"
          label="Username (optional)"
          placeholder="default"
        />
        <TextField
          form={form}
          name="redis_password"
          label="Password (optional)"
          type="password"
          placeholder={keep ?? "••••••••"}
        />
      </div>

      <Disclosure title="TLS" status={tls ? "on" : "off"} defaultOpen={tls} testId="tls-disclosure">
        <CheckboxField form={form} name="tls" label="Use TLS (rediss://)" />
        {/* The PEM is kept across off / on; the backend validates it. */}
        {tls ? (
          <div data-testid="advanced-tls-section">
            <FormField
              control={form.control}
              name="tls_custom_ca"
              render={({ field }) => (
                <FormItem>
                  <div className="flex items-center justify-between gap-2">
                    <FormLabel>Custom CA certificate (PEM, optional)</FormLabel>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      onClick={() => field.onChange("")}
                      disabled={!field.value}
                      className="h-6 px-2 text-xs"
                    >
                      Clear
                    </Button>
                  </div>
                  <FormControl>
                    <Textarea
                      rows={5}
                      placeholder={"-----BEGIN CERTIFICATE-----\n\n-----END CERTIFICATE-----"}
                      className="font-mono text-xs"
                      spellCheck={false}
                      autoComplete="off"
                      {...field}
                    />
                  </FormControl>
                  <p className="text-xs text-(--text2)">
                    Leave empty to use the system trust roots. Paste your provider's CA chain
                    (Upstash, ElastiCache, an internal CA).
                  </p>
                  <FormMessage />
                </FormItem>
              )}
            />
          </div>
        ) : null}
      </Disclosure>

      <Disclosure
        title="SSH tunnel"
        status={ssh ? (sshHost ? `through ${sshHost}` : "on") : "off"}
        defaultOpen={Boolean(ssh)}
        testId="ssh-disclosure"
      >
        <CheckboxField form={form} name="ssh_enabled" label="Use SSH tunnel" />
        {ssh ? <SshFields form={form} keep={keep} /> : null}
      </Disclosure>
    </>
  )
}

function SshFields({ form, keep }: { form: FormApi; keep: string | undefined }) {
  const auth = form.watch("ssh_auth_kind")
  const jump = form.watch("ssh_jump_enabled")
  return (
    <div data-testid="advanced-ssh-section" className="flex flex-col gap-3">
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-[minmax(0,1fr)_6rem_minmax(0,1fr)]">
        <TextField form={form} name="ssh_host" label="SSH host" placeholder="bastion.example.com" />
        <NumberField form={form} name="ssh_port" label="SSH port" min={1} max={65535} />
        <TextField form={form} name="ssh_user" label="SSH user" placeholder="ubuntu" />
      </div>
      <FormField
        control={form.control}
        name="ssh_auth_kind"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Auth method</FormLabel>
            <FormControl>
              <div className="flex gap-4">
                {(
                  [
                    ["keyFile", "Key file"],
                    ["password", "Password"],
                  ] as const
                ).map(([value, label]) => (
                  <label key={value} className="flex items-center gap-2 text-sm">
                    <input
                      type="radio"
                      name="ssh_auth_kind"
                      value={value}
                      checked={field.value === value}
                      onChange={() => field.onChange(value)}
                      className="cursor-pointer"
                    />
                    {label}
                  </label>
                ))}
              </div>
            </FormControl>
          </FormItem>
        )}
      />
      {auth === "keyFile" ? (
        <div className="flex flex-col gap-1">
          <TextField
            form={form}
            name="ssh_key_file_path"
            label="SSH key file path"
            placeholder="~/.ssh/id_ed25519"
          />
          <p className="text-xs text-(--text2)">
            Ranch stores the path only and reads the key when the tunnel opens. Keys with a
            passphrase are not supported yet: use an unencrypted key or a password.
          </p>
        </div>
      ) : (
        <TextField
          form={form}
          name="ssh_password"
          label="SSH password"
          type="password"
          placeholder={keep ?? "••••••••"}
        />
      )}
      <CheckboxField form={form} name="ssh_jump_enabled" label="Use jump host" />
      {jump ? (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-[minmax(0,1fr)_6rem_minmax(0,1fr)]">
          <TextField
            form={form}
            name="ssh_jump_host"
            label="Jump host"
            placeholder="jump.example.com"
          />
          <NumberField form={form} name="ssh_jump_port" label="Jump port" min={1} max={65535} />
          <TextField form={form} name="ssh_jump_user" label="Jump user" placeholder="jumpuser" />
        </div>
      ) : null}
      <p className="text-xs text-(--text2)">
        On first connect Ranch pins the bastion's host key. A different key later blocks the tunnel:
        if it rotated on purpose, remove the connection and add it again.
      </p>
    </div>
  )
}

function SwitchRow({
  id,
  label,
  hint,
  checked,
  onCheckedChange,
}: {
  id: string
  label: string
  hint: string
  checked: boolean
  onCheckedChange: (checked: boolean) => void
}) {
  return (
    <div className="flex items-start justify-between gap-4 rounded-md border border-(--line) p-3">
      <div className="min-w-0">
        <label htmlFor={id} className="text-sm font-medium">
          {label}
        </label>
        <p className="text-xs text-(--text2)">{hint}</p>
      </div>
      <Switch id={id} checked={checked} onCheckedChange={onCheckedChange} />
    </div>
  )
}

/** Step 2's head: testing, what the test found, or why it failed. */
function TestOutcome({
  testing,
  result,
  target,
  tls,
}: {
  testing: boolean
  result: TestConnectionResult | null
  target: { host: string; port: number }
  tls: boolean
}) {
  if (testing || !result) {
    return (
      <p className="flex items-center gap-2 text-sm text-(--text2)" data-testid="test-running">
        <LoaderCircleIcon className="size-4 animate-spin" aria-hidden="true" />
        Testing {target.host}:{target.port}
      </p>
    )
  }
  if (!result.success) {
    const { headline, detail } = plainTestError(result.error, target)
    return (
      <div
        data-testid="test-result"
        data-success="false"
        className="flex items-start gap-2 rounded-md border border-(--danger)/40 bg-(--danger)/10 px-3 py-2 text-sm"
      >
        <TriangleAlertIcon
          className="mt-0.5 size-4 shrink-0 text-(--state-failed-text)"
          aria-hidden="true"
        />
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <p className="font-medium break-words text-(--state-failed-text)">{headline}</p>
          {detail ? <p className="font-mono text-xs break-words text-(--text2)">{detail}</p> : null}
          <p className="text-xs text-(--text2)">Back keeps what you typed so you can fix it.</p>
        </div>
      </div>
    )
  }
  return (
    <div
      data-testid="test-result"
      data-success="true"
      className="flex flex-col gap-2 rounded-md border border-(--line) px-3 py-2 text-sm"
    >
      <p className="flex items-center gap-2 font-medium">
        <CircleCheckIcon className="size-4 shrink-0 text-(--state-completed)" aria-hidden="true" />
        Connected to {target.host}:{target.port}
      </p>
      {result.sshHostKeyPinned?.wasNewPin ? (
        <p className="text-xs break-all text-(--text2)" data-testid="ssh-hostkey-pinned-banner">
          First connect to the bastion: its host key is now pinned (
          <span className="font-mono">{result.sshHostKeyPinned.fingerprint}</span>).
        </p>
      ) : null}
      <ProbeFacts probe={result.probe ?? null} version={result.redisVersion} tls={tls} />
    </div>
  )
}

function ProbeFacts({
  probe,
  version,
  tls,
}: {
  probe: ConnectionProbe | null
  version: string | null
  tls: boolean
}) {
  const facts: Array<[string, ReactNode]> = [
    ["Latency", probe ? `${probe.latencyMs} ms` : "—"],
    ["Redis", version ? `v${version}` : "version not reported"],
    ["TLS", tls ? "on" : "off"],
  ]
  if (probe) {
    const more = probe.queuesFound - probe.queueSample.length
    facts.push(
      [
        "Queues",
        probe.queuesFound === 0 ? (
          "none found yet"
        ) : (
          <span key="queues" className="flex flex-col">
            <span>
              {formatCount(probe.queuesFound)}
              {probe.scanCapped ? " or more" : ""}
            </span>
            <span className="font-mono text-xs break-all text-(--text2)">
              {probe.queueSample.join(", ")}
              {more > 0 ? ` and ${formatCount(more)} more` : ""}
            </span>
          </span>
        ),
      ],
      ["Schedulers", formatCount(probe.schedulers)],
      [
        "Workers",
        probe.workers === null ? "not listed by this server" : formatCount(probe.workers),
      ],
    )
  }
  return (
    <dl
      className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm"
      data-testid="probe-facts"
    >
      {facts.map(([label, value]) => (
        <div key={label} className="contents">
          <dt className="text-(--text2)">{label}</dt>
          <dd className={cn("min-w-0 font-tabular break-words")}>{value}</dd>
        </div>
      ))}
    </dl>
  )
}
