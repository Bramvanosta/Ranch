import { useQueryClient } from "@tanstack/react-query"
import { useMemo, useRef, useState } from "react"
import { toast } from "sonner"

import { displayChord } from "@/commands/chords"
import { CodeBlock } from "@/components/data/CodeBlock"
import { Kbd } from "@/components/data/Kbd"
import { Segmented } from "@/components/data/Segmented"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Sheet } from "@/components/ui/sheet"
import { useQueueMetrics } from "@/features/metrics/useMetrics"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import { type AddJobResult, commands, type QueueRef } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import {
  type AddJobFormErrors,
  type AddJobFormValues,
  addJobSnippet,
  describePlacement,
  EMPTY_ADD_JOB_FORM,
  formatJson,
  validateAddJobForm,
} from "./addJobForm"
import { checkJson } from "./jsonCheck"

/**
 * Enqueue a job from Ranch. The write goes through
 * `add_job`: lock-aware, audited, one atomic script that does what BullMQ's
 * own `Queue.add` does — a real worker picks the job up like any other.
 *
 * A sheet, made to be sure of what goes in:
 *
 *   - the name offers the names this queue already uses;
 *   - the payload editor numbers its lines and says, as you type, "Valid
 *     JSON · 4 keys · 98 bytes" or what is wrong and on which line — and the
 *     button (and ⌘↩) wait for valid JSON;
 *   - the options: attempts, backoff, delay, priority, job ID, keep when
 *     done / failed, LIFO;
 *   - "Same thing in code": the `queue.add(...)` for what is filled in,
 *     live, to copy.
 */
interface AddJobDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  connectionId: string
  queueRef: QueueRef
  queuePaused: boolean
  /** Called once the job exists — the route shows it in its state list. */
  onAdded?: (result: AddJobResult) => void
}

export function AddJobDialog({
  open,
  onOpenChange,
  connectionId,
  queueRef,
  queuePaused,
  onAdded,
}: AddJobDialogProps) {
  const queryClient = useQueryClient()
  const copy = useCopyToClipboard()
  const [values, setValues] = useState<AddJobFormValues>(EMPTY_ADD_JOB_FORM)
  const [errors, setErrors] = useState<AddJobFormErrors>({})
  const [serverError, setServerError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const nameRef = useRef<HTMLInputElement>(null)
  const names = useQueueJobNames(connectionId, queueRef, open)

  function set<K extends keyof AddJobFormValues>(key: K, value: AddJobFormValues[K]) {
    setValues((prev) => ({ ...prev, [key]: value }))
    setErrors((prev) => ({ ...prev, [key]: undefined }))
    setServerError(null)
  }

  function reset() {
    setValues(EMPTY_ADD_JOB_FORM)
    setErrors({})
    setServerError(null)
    setSaving(false)
  }

  const json = useMemo(() => checkJson(values.data), [values.data])
  const snippet = useMemo(() => addJobSnippet(values), [values])
  const canSubmit = json.ok && !saving

  async function submit() {
    if (!canSubmit) return
    const result = validateAddJobForm(values)
    if (!result.ok) {
      setErrors(result.errors)
      return
    }
    setSaving(true)
    const response = await commands.addJob({ connectionId, queueRef, job: result.job })
    setSaving(false)
    if (response.status === "error") {
      // Locked connection, queue gone, an ID already taken — said where the user is looking.
      setServerError(response.error.message)
      return
    }
    void queryClient.invalidateQueries({ queryKey: ["jobs", connectionId] })
    void queryClient.invalidateQueries({ queryKey: ["queues", connectionId] })
    toast.success(`Job #${response.data.jobId} added to ${queueRef.name}`, {
      description: `${result.job.name} · ${response.data.state}`,
    })
    onAdded?.(response.data)
    reset()
    onOpenChange(false)
  }

  return (
    <Sheet
      open={open}
      onOpenChange={(next) => {
        if (!next) reset()
        onOpenChange(next)
      }}
      title={`Add a job to ${queueRef.name}`}
      description="The queue's workers process it like a job added by your application."
      initialFocus={nameRef}
      testId="add-job-dialog"
      className="w-[min(44rem,calc(100vw-2rem))]"
      footer={
        <>
          <Button type="button" size="lg" variant="default" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            type="submit"
            form="add-job-form"
            size="lg"
            variant="primary"
            disabled={!canSubmit}
            data-testid="add-job-submit"
            className="gap-2"
          >
            {saving ? "Adding" : "Add job"}
            <Kbd className="hidden sm:inline-flex">{displayChord("cmd+enter")}</Kbd>
          </Button>
        </>
      }
    >
      <form
        id="add-job-form"
        className="grid gap-4"
        noValidate
        onSubmit={(e) => {
          e.preventDefault()
          void submit()
        }}
        onKeyDown={(e) => {
          // ⌘↩ / Ctrl+↩ adds from anywhere in the form, the editor included.
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault()
            void submit()
          }
        }}
      >
        <Field id="add-job-name" label="Job name" error={errors.name}>
          <Input
            ref={nameRef}
            id="add-job-name"
            value={values.name}
            onChange={(e) => set("name", e.target.value)}
            placeholder="e.g. send-email"
            list="add-job-names"
            autoComplete="off"
            aria-invalid={errors.name ? true : undefined}
          />
          <datalist id="add-job-names" data-testid="add-job-names">
            {names.map((n) => (
              <option key={n} value={n} />
            ))}
          </datalist>
        </Field>

        <Field
          id="add-job-data"
          label="Payload (JSON)"
          error={errors.data ?? (json.ok ? null : `Line ${json.line}: ${json.message}`)}
          action={
            <button
              type="button"
              onClick={() => set("data", formatJson(values.data))}
              disabled={!json.ok}
              className="text-xs text-(--text2) underline-offset-2 hover:underline disabled:opacity-50"
            >
              Format
            </button>
          }
        >
          <JsonEditor
            id="add-job-data"
            value={values.data}
            onChange={(v) => set("data", v)}
            errorLine={json.ok ? null : json.line}
          />
          {json.ok ? (
            <p className="text-xs text-(--state-completed-text)" data-testid="add-job-data-status">
              {json.summary}
            </p>
          ) : null}
        </Field>

        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <Field id="add-job-attempts" label="Attempts" error={errors.attempts}>
            <Input
              id="add-job-attempts"
              inputMode="numeric"
              value={values.attempts}
              onChange={(e) => set("attempts", e.target.value)}
              placeholder="1"
              aria-invalid={errors.attempts ? true : undefined}
            />
          </Field>
          <Field id="add-job-delay" label="Delay (ms)" error={errors.delayMs}>
            <Input
              id="add-job-delay"
              inputMode="numeric"
              value={values.delayMs}
              onChange={(e) => set("delayMs", e.target.value)}
              placeholder="0"
              aria-invalid={errors.delayMs ? true : undefined}
            />
          </Field>
          <Field id="add-job-priority" label="Priority" error={errors.priority}>
            <Input
              id="add-job-priority"
              inputMode="numeric"
              value={values.priority}
              onChange={(e) => set("priority", e.target.value)}
              placeholder="1 = highest"
              aria-invalid={errors.priority ? true : undefined}
            />
          </Field>
          <Field id="add-job-id" label="Job ID" error={errors.jobId}>
            <Input
              id="add-job-id"
              value={values.jobId}
              onChange={(e) => set("jobId", e.target.value)}
              placeholder="automatic"
              autoComplete="off"
              aria-invalid={errors.jobId ? true : undefined}
            />
          </Field>
        </div>

        <div className="grid gap-1.5">
          <span className="text-sm font-medium">Backoff between attempts</span>
          <div className="flex flex-wrap items-center gap-3">
            <Segmented
              label="Backoff"
              value={values.backoffKind}
              onChange={(v) => set("backoffKind", v)}
              options={[
                { value: "none", label: "None", testId: "add-job-backoff-none" },
                { value: "fixed", label: "Fixed", testId: "add-job-backoff-fixed" },
                {
                  value: "exponential",
                  label: "Exponential",
                  testId: "add-job-backoff-exponential",
                },
              ]}
            />
            {values.backoffKind !== "none" ? (
              <span className="flex items-center gap-2">
                <Input
                  id="add-job-backoff-delay"
                  aria-label="Backoff delay (ms)"
                  inputMode="numeric"
                  value={values.backoffDelayMs}
                  onChange={(e) => set("backoffDelayMs", e.target.value)}
                  placeholder="1000"
                  className="w-28"
                  aria-invalid={errors.backoffDelayMs ? true : undefined}
                />
                <span className="text-xs text-(--text2)">ms</span>
              </span>
            ) : null}
          </div>
          {errors.backoffDelayMs ? (
            <p
              className="text-xs text-(--state-failed-text)"
              data-testid="add-job-backoff-delay-error"
            >
              {errors.backoffDelayMs}
            </p>
          ) : null}
        </div>

        <div className="flex flex-wrap gap-x-5 gap-y-2 text-sm">
          <Check
            label="Keep when done"
            checked={values.keepCompleted}
            onChange={(v) => set("keepCompleted", v)}
          />
          <Check
            label="Keep when failed"
            checked={values.keepFailed}
            onChange={(v) => set("keepFailed", v)}
          />
          <Check
            label="Before the jobs already waiting (LIFO)"
            checked={values.lifo}
            onChange={(v) => set("lifo", v)}
          />
        </div>

        <p className="text-xs text-(--text2)" data-testid="add-job-placement">
          {describePlacement(values, queuePaused)}
        </p>

        <section aria-label="Same thing in code" className="grid gap-1.5">
          <h3 className="text-xs font-semibold text-(--text2)">Same thing in code</h3>
          <CodeBlock
            label="The job as queue.add"
            actions={
              <Button
                type="button"
                size="sm"
                variant="default"
                onClick={() => copy(snippet, "code")}
                data-testid="add-job-snippet-copy"
              >
                Copy
              </Button>
            }
          >
            <code data-testid="add-job-snippet">{snippet}</code>
          </CodeBlock>
        </section>

        {serverError ? (
          <p
            role="alert"
            className="rounded-control border border-(--danger)/40 bg-(--danger)/10 p-2 text-xs text-(--state-failed-text)"
          >
            {serverError}
          </p>
        ) : null}
      </form>
    </Sheet>
  )
}

/** Names this queue already uses — its metrics know them. */
function useQueueJobNames(connectionId: string, queueRef: QueueRef, open: boolean): string[] {
  const { data } = useQueueMetrics(connectionId, open ? queueRef : undefined)
  return useMemo(() => {
    const all = [
      ...(data?.usualDurations ?? []).map((u) => u.name),
      ...(data?.failingJobTypes ?? []).map((t) => t.name),
      ...(data?.slowestJobs ?? []).map((j) => j.name),
    ]
    return [...new Set(all)].sort()
  }, [data])
}

/** A textarea with line numbers; the line of a JSON error is marked. */
function JsonEditor({
  id,
  value,
  onChange,
  errorLine,
}: {
  id: string
  value: string
  onChange: (value: string) => void
  errorLine: number | null
}) {
  const gutter = useRef<HTMLDivElement>(null)
  const count = value.split("\n").length
  return (
    <div
      className={cn(
        "grid h-56 grid-cols-[auto_minmax(0,1fr)] overflow-hidden rounded-control border bg-(--field) font-mono text-xs leading-5",
        errorLine !== null ? "border-(--danger)/60" : "border-(--line)",
        "focus-within:ring-3 focus-within:ring-(--accent)/40",
      )}
    >
      <div
        ref={gutter}
        aria-hidden="true"
        className="overflow-hidden border-r border-(--line) py-2 text-right select-none"
        data-testid="add-job-gutter"
      >
        {Array.from({ length: count }, (_, i) => (
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: one number per line, in order.
            key={i}
            className={cn(
              "px-2 tabular-nums",
              errorLine === i + 1
                ? "bg-(--danger)/15 text-(--state-failed-text)"
                : "text-(--text2)",
            )}
            data-error={errorLine === i + 1 ? "true" : undefined}
          >
            {i + 1}
          </div>
        ))}
      </div>
      <textarea
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onScroll={(e) => {
          if (gutter.current) gutter.current.scrollTop = e.currentTarget.scrollTop
        }}
        spellCheck={false}
        wrap="off"
        aria-invalid={errorLine !== null ? true : undefined}
        className="h-full resize-none overflow-auto bg-transparent px-2 py-2 whitespace-pre text-(--text) outline-none"
      />
    </div>
  )
}

function Check({
  label,
  checked,
  onChange,
}: {
  label: string
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <label className="flex items-center gap-2">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="size-4 rounded border border-(--line)"
      />
      {label}
    </label>
  )
}

function Field({
  id,
  label,
  error,
  action,
  children,
}: {
  id: string
  label: string
  error?: string | null
  action?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <div className="grid content-start gap-1.5">
      <div className="flex items-baseline justify-between">
        <Label htmlFor={id}>{label}</Label>
        {action}
      </div>
      {children}
      {error ? (
        <p className="text-xs text-(--state-failed-text)" data-testid={`${id}-error`}>
          {error}
        </p>
      ) : null}
    </div>
  )
}
