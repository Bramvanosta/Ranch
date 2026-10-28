import type { RefObject } from "react"

import { Button } from "@/components/ui/button"
import { FormControl, FormField, FormItem, FormLabel, FormMessage } from "@/components/ui/form"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import type { BullmqRedisEndpoint, EndpointSchema } from "@/lib/tauri/bindings"

import {
  CheckboxField,
  Disclosure,
  type FormApi,
  NumberField,
  TextField,
} from "./ConnectionFormFields"

export interface ServerStepProps {
  form: FormApi
  urlRef: RefObject<HTMLInputElement | null>
  isEdit: boolean
  /** The saved endpoint of the connection being edited, for where its passwords may go. */
  saved: BullmqRedisEndpoint | null
  /** What the backend lists for its endpoint: the fields' labels and which are required. */
  schema: EndpointSchema | undefined
  urlInput: string
  urlError: string | null
  onUrl: (raw: string) => void
}

/**
 * A field's label as the backend's endpoint schema gives it, "(optional)"
 * when it is not required; `fallback` until the schema is listed.
 */
function labelOf(schema: EndpointSchema | undefined, key: string, fallback: string): string {
  const field = schema?.fields.find((f) => f.key === key)
  if (!field) return fallback
  return field.required ? field.label : `${field.label} (optional)`
}

/**
 * Step 1 of the connection sheet for BullMQ on Redis: where the server is.
 * A pasted URL fills the fields; TLS and the SSH tunnel sit behind
 * disclosures, open when in use.
 */
export function ServerStep({
  form,
  urlRef,
  isEdit,
  saved,
  schema,
  urlInput,
  urlError,
  onUrl,
}: ServerStepProps) {
  const tls = form.watch("tls")
  const ssh = form.watch("ssh_enabled")
  const sshHost = form.watch("ssh_host")
  // A saved password is only sent back to the server (or bastion) it was
  // saved for: once the endpoint changes, the backend wants it typed again.
  const host = form.watch("redis_host")
  const port = form.watch("redis_port")
  const sshPort = form.watch("ssh_port")
  const sshUser = form.watch("ssh_user")
  const redisMoved =
    saved !== null &&
    (host.trim().toLowerCase() !== saved.host.toLowerCase() ||
      Number(port) !== saved.port ||
      tls !== saved.tls)
  const sshMoved =
    saved !== null &&
    (saved.sshTunnel === null ||
      (sshHost ?? "").trim().toLowerCase() !== saved.sshTunnel.host.toLowerCase() ||
      Number(sshPort) !== saved.sshTunnel.port ||
      sshUser !== saved.sshTunnel.user)
  const keepFor = (moved: boolean) =>
    !isEdit ? undefined : moved ? "Required: the server changed" : "Leave blank to keep current"
  const keep = keepFor(redisMoved)
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
        <TextField
          form={form}
          name="redis_host"
          label={labelOf(schema, "host", "Host")}
          placeholder="localhost"
        />
        <NumberField
          form={form}
          name="redis_port"
          label={labelOf(schema, "port", "Port")}
          min={1}
          max={65535}
        />
        <NumberField
          form={form}
          name="redis_db"
          label={labelOf(schema, "db", "DB index")}
          min={0}
          max={15}
        />
      </div>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <TextField
          form={form}
          name="redis_username"
          label={labelOf(schema, "username", "Username (optional)")}
          placeholder="default"
        />
        <TextField
          form={form}
          name="redis_password"
          label={labelOf(schema, "password", "Password (optional)")}
          type="password"
          placeholder={keep}
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
        {ssh ? <SshFields form={form} keep={keepFor(sshMoved)} /> : null}
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
          placeholder={keep}
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
