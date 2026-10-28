import type * as React from "react"

import { cn } from "@/lib/utils"

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        "h-control w-full min-w-0 rounded-control border border-(--line) bg-(--field) px-2.5 text-sm text-(--text) transition-[color,box-shadow] outline-none selection:bg-(--accent)/30 file:inline-flex file:h-6 file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-(--text) placeholder:text-(--text2) disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-60",
        "focus-visible:border-(--accent) focus-visible:ring-3 focus-visible:ring-(--accent)/50",
        "aria-invalid:border-(--danger) aria-invalid:ring-3 aria-invalid:ring-(--danger)/30",
        className,
      )}
      {...props}
    />
  )
}

export { Input }
