import { useState } from "react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Sheet } from "@/components/ui/sheet"
import { Textarea } from "@/components/ui/textarea"
import { useRemoveScheduler, useUpsertScheduler } from "@/data/schedulers/mutations"
import { useSchedulePreview } from "@/data/schedulers/queries"
import { formatJson } from "@/domain/format"
import { useDebouncedValue } from "@/hooks/useDebouncedValue"
import type { CronJob, QueueId } from "@/lib/tauri/bindings"
import { formatMoment } from "@/lib/time"
import { cn } from "@/lib/utils"
import {
  describeSchedule,
  emptySchedulerForm,
  INTERVAL_UNITS,
  type IntervalUnit,
  type ScheduleKind,
  type SchedulerFormErrors,
  type SchedulerFormValues,
  scheduleOf,
  schedulerFormFrom,
  validateSchedulerForm,
} from "./schedulerForm"

/**
 * Create or edit a job scheduler: id, strategy (cron
 * pattern + timezone, or a fixed interval), job name, JSON data template.
 * The **next 5 runs** are computed by the backend with the parser that will
 * compute the scheduler's first run (what you preview is what gets written),
 * and an invalid pattern or timezone is reported under the field as you type.
 * Editing keeps the id (it is BullMQ's key); Delete asks once more.
 */
interface SchedulerSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  connectionId: string
  queueId: QueueId
  queueName: string
  /** The scheduler to edit; omit to create one. */
  scheduler?: CronJob | null
}

const COMMON_TIMEZONES = [
  "UTC",
  "Europe/Paris",
  "Europe/London",
  "Europe/Berlin",
  "America/New_York",
  "America/Chicago",
  "America/Los_Angeles",
  "America/Sao_Paulo",
  "Asia/Tokyo",
  "Asia/Singapore",
  "Asia/Kolkata",
  "Australia/Sydney",
]

function localTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC"
  } catch {
    return "UTC"
  }
}

export function SchedulerSheet(props: SchedulerSheetProps) {
  // Remount per target so the form starts from the right values.
  const formKey = props.open ? (props.scheduler?.key ?? "__new__") : "__closed__"
  return <SchedulerSheetForm key={formKey} {...props} />
}

function SchedulerSheetForm({
  open,
  onOpenChange,
  connectionId,
  queueId,
  queueName,
  scheduler,
}: SchedulerSheetProps) {
  const editing = scheduler != null
  const [values, setValues] = useState<SchedulerFormValues>(() =>
    scheduler ? schedulerFormFrom(scheduler, localTimezone()) : emptySchedulerForm(localTimezone()),
  )
  const [errors, setErrors] = useState<SchedulerFormErrors>({})
  const [serverError, setServerError] = useState<string | null>(null)
  const [confirmingDelete, setConfirmingDelete] = useState(false)
  const upsert = useUpsertScheduler()
  const remove = useRemoveScheduler()
  const busy = upsert.isPending || remove.isPending

  function set<K extends keyof SchedulerFormValues>(key: K, value: SchedulerFormValues[K]) {
    setValues((prev) => ({ ...prev, [key]: value }))
    setErrors({})
    setServerError(null)
    setConfirmingDelete(false)
  }

  // The preview follows the typing, a beat behind.
  const schedule = useDebouncedValue(scheduleOf(values), 300)
  const runs = useSchedulePreview(schedule, open)
  const scheduleError =
    runs.isError && runs.error instanceof Error && schedule !== null ? runs.error.message : null

  function save() {
    const result = validateSchedulerForm(values)
    if (!result.ok) {
      setErrors(result.errors)
      return
    }
    upsert.mutate(
      { connectionId, queueId, scheduler: result.scheduler },
      {
        onSuccess: (saved) => {
          toast.success(`Scheduler ${saved.schedulerId} ${editing ? "updated" : "created"}`, {
            description: `${describeSchedule(result.scheduler.schedule)} · next run ${formatMoment(saved.nextRunMs)}`,
          })
          onOpenChange(false)
        },
        onError: (err) => setServerError(err.message),
      },
    )
  }

  function destroy() {
    if (!scheduler) return
    if (!confirmingDelete) {
      setConfirmingDelete(true)
      return
    }
    remove.mutate(
      { connectionId, queueId, schedulerKey: scheduler.key },
      {
        onSuccess: () => {
          toast.success(`Scheduler ${scheduler.key} removed`)
          onOpenChange(false)
        },
        onError: (err) => setServerError(err.message),
      },
    )
  }

  let dataError = errors.data ?? null
  if (!dataError && values.data.trim() !== "") {
    try {
      JSON.parse(values.data)
    } catch (e) {
      dataError = e instanceof Error ? e.message : "Not valid JSON"
    }
  }

  return (
    <Sheet
      open={open}
      onOpenChange={(next) => !busy && onOpenChange(next)}
      testId="scheduler-sheet"
      className="w-[min(36rem,calc(100vw-2rem))]"
      title={editing ? `Edit scheduler ${scheduler.key}` : `New scheduler in ${queueName}`}
      description="A scheduler adds a job to the queue at each run; the queue's workers process it like any other."
    >
      <form
        className="grid gap-4"
        noValidate
        onSubmit={(e) => {
          e.preventDefault()
          save()
        }}
      >
        <div className="grid grid-cols-2 gap-3">
          <Field id="scheduler-id" label="Scheduler id" error={errors.id}>
            <Input
              id="scheduler-id"
              value={values.id}
              onChange={(e) => set("id", e.target.value)}
              placeholder="e.g. nightly-report"
              // The id is BullMQ's key: another id is another scheduler.
              readOnly={editing}
              // Read-only = not a stop for the keyboard: the dialog would
              // otherwise open with this field focused and its text selected,
              // looking editable.
              tabIndex={editing ? -1 : undefined}
              className={cn(editing && "cursor-default bg-(--surf)/40 text-(--text2)")}
              autoFocus={!editing}
            />
          </Field>
          <Field id="scheduler-job-name" label="Job name" error={errors.jobName}>
            <Input
              id="scheduler-job-name"
              value={values.jobName}
              onChange={(e) => set("jobName", e.target.value)}
              placeholder="e.g. build-report"
            />
          </Field>
        </div>

        <fieldset className="grid gap-2">
          <legend className="mb-1.5 text-sm font-medium">Runs</legend>
          <div className="flex gap-4 text-sm">
            {(
              [
                ["pattern", "On a cron pattern"],
                ["every", "At a fixed interval"],
              ] as Array<[ScheduleKind, string]>
            ).map(([kind, label]) => (
              <label key={kind} className="flex items-center gap-1.5">
                <input
                  type="radio"
                  name="scheduler-kind"
                  checked={values.kind === kind}
                  onChange={() => set("kind", kind)}
                />
                {label}
              </label>
            ))}
          </div>

          {values.kind === "pattern" ? (
            <div className="grid grid-cols-[1fr_12rem] gap-3">
              <Field
                id="scheduler-pattern"
                label="Cron pattern"
                error={errors.pattern ?? scheduleError}
                hint="minute hour day month weekday, or 6 fields, seconds first"
              >
                <Input
                  id="scheduler-pattern"
                  value={values.pattern}
                  onChange={(e) => set("pattern", e.target.value)}
                  spellCheck={false}
                  className="font-mono"
                  aria-invalid={scheduleError ? true : undefined}
                />
              </Field>
              <Field
                id="scheduler-tz"
                label="Timezone"
                hint={
                  values.tz.trim() === ""
                    ? editing && !scheduler?.tz
                      ? "Read in each worker's local time today. Saved empty, it runs in UTC."
                      : "Empty = UTC"
                    : undefined
                }
              >
                <Input
                  id="scheduler-tz"
                  value={values.tz}
                  onChange={(e) => set("tz", e.target.value)}
                  list="scheduler-tz-options"
                  placeholder="e.g. Europe/Paris"
                  spellCheck={false}
                />
                <datalist id="scheduler-tz-options">
                  {COMMON_TIMEZONES.map((tz) => (
                    <option key={tz} value={tz} />
                  ))}
                </datalist>
              </Field>
            </div>
          ) : (
            <Field id="scheduler-every" label="Every" error={errors.every ?? scheduleError}>
              <div className="flex gap-2">
                <Input
                  id="scheduler-every"
                  inputMode="decimal"
                  value={values.everyValue}
                  onChange={(e) => set("everyValue", e.target.value)}
                  className="w-28"
                />
                <select
                  aria-label="Interval unit"
                  value={values.everyUnit}
                  onChange={(e) => set("everyUnit", e.target.value as IntervalUnit)}
                  className="h-9 rounded-md border border-(--line) bg-transparent px-2 text-sm dark:bg-(--field)/30"
                >
                  {INTERVAL_UNITS.map((u) => (
                    <option key={u.value} value={u.value}>
                      {u.label}
                    </option>
                  ))}
                </select>
              </div>
            </Field>
          )}

          <div
            className="rounded-md border border-(--line)/60 bg-(--surf)/30 p-3 text-xs"
            data-testid="scheduler-next-runs"
            aria-live="polite"
          >
            <p className="mb-1 font-medium text-(--text2)">Next runs</p>
            {scheduleError ? (
              <p className="text-(--text2)">Fix the schedule above</p>
            ) : runs.data ? (
              <ol className="grid gap-0.5 font-mono tabular-nums">
                {runs.data.map((ms, i) => (
                  <li key={ms} title={new Date(ms).toISOString()}>
                    {values.kind === "every" && i === 0 ? "now · " : ""}
                    {formatMoment(ms)}
                  </li>
                ))}
              </ol>
            ) : (
              <p className="text-(--text2)">
                {schedule === null ? "Enter a schedule" : "Computing"}
              </p>
            )}
          </div>
        </fieldset>

        <Field
          id="scheduler-data"
          label="Job data template (JSON)"
          error={dataError}
          action={
            <button
              type="button"
              onClick={() => set("data", formatJson(values.data))}
              disabled={dataError !== null}
              className="text-xs text-(--text2) underline-offset-2 hover:underline disabled:opacity-50"
            >
              Format
            </button>
          }
        >
          <Textarea
            id="scheduler-data"
            value={values.data}
            onChange={(e) => set("data", e.target.value)}
            spellCheck={false}
            className="max-h-56 min-h-24 overflow-auto font-mono text-xs"
          />
        </Field>

        {serverError ? (
          <p
            role="alert"
            className="rounded-md border border-(--danger)/40 bg-(--danger)/10 p-2 text-xs text-(--state-failed-text)"
          >
            {serverError}
          </p>
        ) : null}

        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-(--line) pt-3">
          {editing ? (
            <Button
              type="button"
              variant={confirmingDelete ? "danger" : "ghost"}
              onClick={destroy}
              disabled={busy}
              data-testid="scheduler-delete"
              className={cn(
                !confirmingDelete && "text-(--state-failed-text) hover:text-(--state-failed-text)",
              )}
            >
              {confirmingDelete ? "Remove for good, its pending job too" : "Remove"}
            </Button>
          ) : (
            <span />
          )}
          <div className="flex gap-2">
            <Button
              type="button"
              variant="ghost"
              onClick={() => onOpenChange(false)}
              disabled={busy}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              type="submit"
              disabled={busy || scheduleError !== null}
              data-testid="scheduler-save"
            >
              {upsert.isPending ? "Saving" : editing ? "Save changes" : "Create scheduler"}
            </Button>
          </div>
        </div>
      </form>
    </Sheet>
  )
}

function Field({
  id,
  label,
  error,
  hint,
  action,
  children,
}: {
  id: string
  label: string
  error?: string | null
  hint?: string
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
      ) : hint ? (
        <p className="text-2xs text-(--text2)">{hint}</p>
      ) : null}
    </div>
  )
}
