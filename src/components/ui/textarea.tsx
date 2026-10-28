import type * as React from "react"

import { cn } from "@/lib/utils"

function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        "flex field-sizing-content min-h-16 w-full resize-none rounded-control border border-(--line) bg-(--field) px-2.5 py-2 text-sm text-(--text) transition-[color,box-shadow] outline-none placeholder:text-(--text2) focus-visible:border-(--accent) focus-visible:ring-3 focus-visible:ring-(--accent)/50 disabled:cursor-not-allowed disabled:opacity-60 aria-invalid:border-(--danger) aria-invalid:ring-3 aria-invalid:ring-(--danger)/30",
        className,
      )}
      {...props}
    />
  )
}

export { Textarea }
