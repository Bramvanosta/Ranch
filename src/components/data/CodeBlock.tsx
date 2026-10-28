import { cn } from "@/lib/utils"

/**
 * Code and payloads: mono on `--field`, syntax colours
 * from the `--syntax-*` tokens. Long lines scroll inside the block; they are
 * never cut. `actions` sit in the top-right corner (e.g. a Copy button).
 */
export function CodeBlock({
  children,
  label,
  actions,
  className,
  ...props
}: {
  children: React.ReactNode
  /** The accessible name of the block, e.g. "Payload". */
  label: string
  actions?: React.ReactNode
  className?: string
} & Omit<React.ComponentProps<"figure">, "children">) {
  return (
    <figure
      aria-label={label}
      data-slot="code-block"
      className={cn("relative rounded-card border border-(--line) bg-(--field)", className)}
      {...props}
    >
      {actions ? <div className="absolute top-2 right-2 z-10 flex gap-1.5">{actions}</div> : null}
      <pre
        className={cn(
          "overflow-x-auto p-3 font-mono text-xs leading-relaxed text-(--text)",
          actions && "pr-28",
        )}
      >
        {children}
      </pre>
    </figure>
  )
}

const TOKEN = {
  key: "text-(--syntax-key)",
  string: "text-(--syntax-string)",
  literal: "text-(--syntax-literal)",
  punctuation: "text-(--text2)",
} as const

/** One coloured token inside a `CodeBlock`. */
export function CodeToken({
  kind,
  children,
  ...props
}: { kind: keyof typeof TOKEN; children: React.ReactNode } & React.ComponentProps<"span">) {
  return (
    <span className={TOKEN[kind]} {...props}>
      {children}
    </span>
  )
}
