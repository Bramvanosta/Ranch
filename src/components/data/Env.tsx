import { cn } from "@/lib/utils"

/**
 * Environment markers. The colour comes from the
 * connection's environment token (`env-production`, `env-custom-3`…, see
 * `features/connections/palette.ts`); it is the only place those colours appear.
 */

const envColor = (token: string) => `var(--${token})`

/** The environment dot, with its name as the accessible label. */
export function EnvDot({
  color,
  label,
  decorative = false,
  className,
  ...props
}: {
  /** An `--env-*` token name without the dashes, e.g. `env-production`. */
  color: string
  /** Required unless the environment's name is already written next to the dot. */
  label?: string
  /** Hide the dot from screen readers: the name is next to it. */
  decorative?: boolean
  className?: string
} & Omit<React.ComponentProps<"span">, "color">) {
  return (
    <span
      {...(decorative
        ? { "aria-hidden": true }
        : { role: "img", "aria-label": `Environment: ${label ?? color}` })}
      title={label}
      data-slot="env-dot"
      {...props}
      className={cn("inline-block size-2 shrink-0 rounded-full", className)}
      style={{ backgroundColor: envColor(color) }}
    />
  )
}

/** The environment as a tag: dot and name, tinted with the environment colour. */
export function EnvTag({
  color,
  label,
  className,
  ...props
}: {
  color: string
  label: string
  className?: string
} & Omit<React.ComponentProps<"span">, "color">) {
  return (
    <span
      data-slot="env-tag"
      title={`Environment: ${label}`}
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 rounded-tag border px-1.5 py-px text-2xs font-semibold whitespace-nowrap text-(--text)",
        className,
      )}
      style={{
        borderColor: `color-mix(in oklab, ${envColor(color)} 45%, transparent)`,
        backgroundColor: `color-mix(in oklab, ${envColor(color)} 12%, transparent)`,
      }}
      {...props}
    >
      <span
        aria-hidden="true"
        className="size-1.5 shrink-0 rounded-full"
        style={{ backgroundColor: envColor(color) }}
      />
      {label}
    </span>
  )
}
