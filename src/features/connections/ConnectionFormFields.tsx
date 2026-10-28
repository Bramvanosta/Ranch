import { ChevronRightIcon } from "lucide-react"
import type { ReactNode } from "react"
import type { useForm } from "react-hook-form"

import { FormControl, FormField, FormItem, FormLabel, FormMessage } from "@/components/ui/form"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"

import type { ConnectionFormInput, ConnectionFormValues } from "./connectionFormSchema"

/** The fields of the connection form, bound to its react-hook-form instance. */

export type FormApi = ReturnType<
  typeof useForm<ConnectionFormInput, undefined, ConnectionFormValues>
>

export function NumberField({
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

export function TextField({
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

export function CheckboxField({
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
export function Disclosure({
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

export function SwitchRow({
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
