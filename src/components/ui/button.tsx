import { Button as ButtonPrimitive } from "@base-ui/react/button"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/lib/utils"

/**
 * The one button.
 *
 * - `default`: a neutral surface button; `primary`: the accent (orange), for the
 *   one main action of a screen or sheet; `ai`: actions that hand work to an
 *   agent (violet); `danger`: destructive confirmations only; `ghost`: toolbar
 *   and row actions.
 * - Sizes are the spec's 28 / 30 / 34 px (`sm` / `default` / `lg`), plus square
 *   `icon-*` versions.
 * - Labels never wrap or shrink, and never end with "…": an action
 *   that opens a confirmation says what it does, a pending one shows a spinner.
 */
const buttonVariants = cva(
  "group/button inline-flex shrink-0 items-center justify-center gap-1.5 rounded-control border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-3 focus-visible:ring-(--accent)/50 disabled:pointer-events-none disabled:opacity-60 aria-invalid:border-(--danger) [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default:
          "border-(--line) bg-(--surf) text-(--text) hover:bg-(--surf2) aria-expanded:bg-(--surf2)",
        primary:
          "border-transparent bg-(--accent) text-(--accent-text) hover:bg-(--accent)/88 aria-expanded:bg-(--accent)/88",
        ai: "border-transparent bg-(--ai) text-(--ai-text) hover:bg-(--ai)/88 aria-expanded:bg-(--ai)/88",
        danger:
          "border-transparent bg-(--danger) text-(--danger-text) hover:bg-(--danger)/88 focus-visible:ring-(--danger)/50",
        ghost:
          "border-transparent bg-transparent text-(--text) hover:bg-(--surf2) aria-expanded:bg-(--surf2)",
      },
      size: {
        sm: "h-control-sm px-2.5 text-xs [&_svg:not([class*='size-'])]:size-3.5",
        default: "h-control px-3 text-sm",
        lg: "h-control-lg px-4 text-sm",
        "icon-sm": "size-control-sm [&_svg:not([class*='size-'])]:size-3.5",
        icon: "size-control",
        "icon-lg": "size-control-lg",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
)

type ButtonProps = ButtonPrimitive.Props &
  VariantProps<typeof buttonVariants> & {
    /** A key hint shown after the label, in mono (e.g. "⇧⌘R"). */
    shortcut?: string
  }

function Button({
  className,
  variant = "default",
  size = "default",
  shortcut,
  children,
  ...props
}: ButtonProps) {
  return (
    <ButtonPrimitive
      data-slot="button"
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    >
      {children}
      {shortcut ? (
        <kbd data-slot="button-shortcut" className="font-mono text-2xs font-normal">
          {shortcut}
        </kbd>
      ) : null}
    </ButtonPrimitive>
  )
}

export { Button, buttonVariants }
