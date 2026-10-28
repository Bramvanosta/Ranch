import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { cn } from "@/lib/utils"

import { ENV_COLOR_PALETTE, type EnvColorToken } from "./palette"

interface EnvironmentColorPickerProps {
  value: EnvColorToken
  onChange: (value: EnvColorToken) => void
}

/**
 * Curated environment color picker.
 *
 * Renders the 10-color palette from `palette.ts` as a horizontal RadioGroup.
 * Each swatch is a colored disc with a hidden radio for keyboard nav and
 * accessibility. The selected token is the contract sent to the backend's
 * `add_connection` command's `env_color` field.
 */
export function EnvironmentColorPicker({ value, onChange }: EnvironmentColorPickerProps) {
  return (
    <RadioGroup
      value={value}
      onValueChange={(v: string) => onChange(v as EnvColorToken)}
      className="grid grid-cols-5 gap-2"
      aria-label="Environment color"
    >
      {ENV_COLOR_PALETTE.map((opt) => (
        <label
          key={opt.token}
          htmlFor={`env-color-${opt.token}`}
          className={cn(
            "group relative flex cursor-pointer flex-col items-center gap-1.5 rounded-md border border-transparent p-2",
            "hover:bg-(--surf2)",
            value === opt.token && "border-(--accent) bg-(--surf2)",
          )}
          title={opt.name}
        >
          <span
            aria-hidden="true"
            className={cn(
              "h-6 w-6 rounded-full border border-(--line)/40 ring-2 ring-transparent transition-shadow",
              value === opt.token && "ring-(--accent)/60",
            )}
            style={{ backgroundColor: `var(--${opt.token})` }}
          />
          <RadioGroupItem value={opt.token} id={`env-color-${opt.token}`} className="sr-only" />
          <span className="text-xs leading-none text-(--text2)">{opt.name}</span>
        </label>
      ))}
    </RadioGroup>
  )
}
