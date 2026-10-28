import type { ReactNode } from "react"

/** A titled part of a Settings page. */
export function SettingsPane({
  title,
  description,
  testId,
  children,
}: {
  title: string
  description?: ReactNode
  testId?: string
  children: ReactNode
}) {
  return (
    <section className="flex flex-col gap-4" data-testid={testId}>
      <header className="flex flex-col gap-1">
        <h2 className="text-lg font-semibold">{title}</h2>
        {description ? <p className="text-sm text-(--text2)">{description}</p> : null}
      </header>
      {children}
    </section>
  )
}

/** A setting on one line: what it is on the left, its control on the right. */
export function SettingRow({
  label,
  hint,
  control,
  testId,
}: {
  label: string
  hint?: ReactNode
  control: ReactNode
  testId?: string
}) {
  return (
    <div
      className="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 border-b border-(--line) py-3 last:border-b-0"
      data-testid={testId}
    >
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-sm font-medium">{label}</span>
        {hint ? <span className="text-xs text-(--text2)">{hint}</span> : null}
      </div>
      <div className="shrink-0">{control}</div>
    </div>
  )
}
