import { cn } from "@/lib/utils"

/**
 * A key hint: mono, on a quiet surface. `label` spells
 * the keys out for screen readers when the glyphs alone are cryptic
 * ("⇧⌘U" → "Shift Command U").
 */
export function Kbd({
  children,
  label,
  className,
}: {
  children: React.ReactNode
  label?: string
  className?: string
}) {
  return (
    <kbd
      data-slot="kbd"
      className={cn(
        "inline-flex shrink-0 items-center rounded-tag border border-(--line) bg-(--surf) px-1 py-px font-mono text-2xs font-normal whitespace-nowrap text-(--text2)",
        className,
      )}
    >
      {label ? (
        <>
          <span aria-hidden="true">{children}</span>
          <span className="sr-only">{label}</span>
        </>
      ) : (
        children
      )}
    </kbd>
  )
}
